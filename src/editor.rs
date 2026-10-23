//! Page layout, painting and text editing for a [`Document`].
//!
//! egui only ships a plain-text editor, so this module does its own rich-text
//! editing on top of egui's text layout (`Galley`): each paragraph is laid out
//! separately, then placed onto pages.

use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{
    self, Align as EAlign, Color32, CornerRadius, FontId, Galley, Key, Modifiers, Pos2, Rect,
    Sense, Stroke, StrokeKind, Ui, Vec2, pos2, text::CCursor, text::CharIndex, text::LayoutJob,
    text::TextFormat, vec2,
};

use crate::doc::{Document, Fmt, Pos};
use crate::theme::{Align, FontMap, Theme, color};

/// Screen points per inch at 100% zoom.
const DPI: f32 = 96.0;

struct ParaLayout {
    galley: Arc<Galley>,
    /// Galley anchor in canvas space.
    origin: Pos2,
}

struct Layout {
    paras: Vec<ParaLayout>,
    pages: Vec<Rect>,
    size: Vec2,
}

#[derive(Clone)]
struct Snapshot {
    doc: Document,
    cursor: Pos,
    anchor: Pos,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum EditKind {
    Typing,
    Other,
}

pub struct Editor {
    pub doc: Document,
    cursor: Pos,
    anchor: Pos,
    /// Formatting toggled with no selection, applied to the next typed text.
    pending: Option<Fmt>,
    /// Horizontal position to keep when moving up/down, in canvas space.
    goal_x: Option<f32>,
    prefer_next_row: bool,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_edit: Option<EditKind>,
    blink_start: f64,
    scroll_to_cursor: bool,
    pub zoom: f32,
    pub dirty: bool,
    pub page_count: usize,
    /// Goes up on every change, so others can tell the document changed.
    pub revision: u64,
    /// Page the caret is on, counting from 1.
    pub caret_page: usize,
}

/// Text for a page's header and footer: (page, pages) → (header, footer).
pub type PageText<'a> = &'a dyn Fn(usize, usize) -> (String, String);

impl Editor {
    pub fn new(doc: Document) -> Self {
        Self {
            doc,
            cursor: Pos::default(),
            anchor: Pos::default(),
            pending: None,
            goal_x: None,
            prefer_next_row: true,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
            blink_start: 0.0,
            scroll_to_cursor: false,
            zoom: 1.0,
            dirty: false,
            page_count: 1,
            revision: 0,
            caret_page: 1,
        }
    }

    fn selection(&self) -> Option<(Pos, Pos)> {
        (self.cursor != self.anchor)
            .then(|| (self.cursor.min(self.anchor), self.cursor.max(self.anchor)))
    }

    // ---- Editing ----------------------------------------------------------

    fn checkpoint(&mut self, kind: EditKind) {
        if !(kind == EditKind::Typing && self.last_edit == Some(EditKind::Typing)) {
            self.undo.push(Snapshot {
                doc: self.doc.clone(),
                cursor: self.cursor,
                anchor: self.anchor,
            });
            if self.undo.len() > 500 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_edit = Some(kind);
        self.dirty = true;
        self.revision += 1;
    }

    fn restore(&mut self, from_undo: bool) {
        let (src, dst) = if from_undo {
            (&mut self.undo, &mut self.redo)
        } else {
            (&mut self.redo, &mut self.undo)
        };
        if let Some(s) = src.pop() {
            dst.push(Snapshot {
                doc: self.doc.clone(),
                cursor: self.cursor,
                anchor: self.anchor,
            });
            self.doc = s.doc;
            self.cursor = s.cursor;
            self.anchor = s.anchor;
            self.last_edit = None;
            self.dirty = true;
            self.revision += 1;
        }
    }

    fn set_cursor(&mut self, p: Pos, extend: bool) {
        self.cursor = self.doc.clamp(p);
        if !extend {
            self.anchor = self.cursor;
        }
        self.pending = None;
        self.last_edit = None;
        self.scroll_to_cursor = true;
    }

    fn delete_selection(&mut self) -> bool {
        if let Some((a, b)) = self.selection() {
            self.cursor = self.doc.delete(a, b);
            self.anchor = self.cursor;
            true
        } else {
            false
        }
    }

    fn insert_text(&mut self, text: &str, theme: &Theme, kind: EditKind) {
        self.checkpoint(kind);
        let fmt = self
            .pending
            .unwrap_or_else(|| self.doc.fmt_at(self.selection().map_or(self.cursor, |s| s.0)));
        self.delete_selection();
        let next = theme.next_style(&self.doc.paragraphs[self.cursor.para].style);
        self.cursor = self.doc.insert(self.cursor, text, fmt, &next);
        self.anchor = self.cursor;
        self.scroll_to_cursor = true;
    }

    fn new_paragraph(&mut self, theme: &Theme) {
        self.checkpoint(EditKind::Other);
        self.delete_selection();
        let style = &self.doc.paragraphs[self.cursor.para].style;
        // Enter at the end moves on to the style's `next`; in the middle it keeps the style.
        let next = if self.cursor.ch == self.doc.paragraphs[self.cursor.para].len() {
            theme.next_style(style)
        } else {
            style.clone()
        };
        self.cursor = self.doc.split(self.cursor, &next);
        self.anchor = self.cursor;
        self.scroll_to_cursor = true;
    }

    fn delete_back(&mut self, to: Pos) {
        self.checkpoint(EditKind::Other);
        if !self.delete_selection() && to != self.cursor {
            self.cursor = self.doc.delete(to, self.cursor);
            self.anchor = self.cursor;
        }
        self.scroll_to_cursor = true;
    }

    fn delete_forward(&mut self, to: Pos) {
        self.checkpoint(EditKind::Other);
        if !self.delete_selection() && to != self.cursor {
            self.doc.delete(self.cursor, to);
        }
    }

    /// Inserts text at the caret (replacing any selection), as one undo step.
    pub fn insert_at_cursor(&mut self, text: &str, theme: &Theme) {
        self.insert_text(text, theme, EditKind::Other);
    }

    /// Swaps in a whole new document, as one undo step.
    pub fn replace_document(&mut self, doc: Document) {
        self.checkpoint(EditKind::Other);
        self.doc = doc;
        self.cursor = self.doc.clamp(self.cursor);
        self.anchor = self.cursor;
    }

    /// Moves the caret to the start of a paragraph and scrolls to it.
    pub fn goto(&mut self, para: usize) {
        self.set_cursor(Pos::new(para, 0), false);
        self.goal_x = None;
    }

    pub fn cursor(&self) -> Pos {
        self.cursor
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|(a, b)| self.doc.text(a, b))
    }

    pub fn select_all(&mut self) {
        self.anchor = Pos::default();
        self.cursor = self.doc.end();
    }

    pub fn undo(&mut self) {
        self.restore(true);
    }

    pub fn redo(&mut self) {
        self.restore(false);
    }

    /// Toggles bold/italic/underline on the selection, or for the next typed text.
    pub fn toggle(&mut self, get: fn(&Fmt) -> bool, set: fn(&mut Fmt, bool)) {
        if let Some((a, b)) = self.selection() {
            self.checkpoint(EditKind::Other);
            let on = !self.doc.all_fmt(a, b, get);
            self.doc.for_each_fmt(a, b, |f| set(f, on));
        } else {
            let mut fmt = self.pending.unwrap_or_else(|| self.doc.fmt_at(self.cursor));
            let on = !get(&fmt);
            set(&mut fmt, on);
            self.pending = Some(fmt);
        }
    }

    pub fn set_style(&mut self, style: &str) {
        self.checkpoint(EditKind::Other);
        let (a, b) = self.selection().unwrap_or((self.cursor, self.cursor));
        for p in &mut self.doc.paragraphs[a.para..=b.para] {
            p.style = style.to_owned();
        }
    }

    pub fn reset_caret(&mut self, now: f64) {
        self.blink_start = now;
    }

    // ---- Layout -----------------------------------------------------------

    fn layout(&self, ui: &Ui, theme: &Theme, fonts: &FontMap, min_width: f32) -> Layout {
        let inch = DPI * self.zoom;
        let pt = inch / 72.0;
        let page = &theme.page;
        let (page_w, page_h) = (page.width * inch, page.height * inch);
        let (mt, mb) = (page.margin_top * inch, page.margin_bottom * inch);
        let (ml, mr) = (page.margin_left * inch, page.margin_right * inch);
        let gap = page.gap * inch;
        let content_w = (page_w - ml - mr).max(20.0);
        let canvas_w = min_width.max(page_w + 2.0 * gap);
        let page_x = ((canvas_w - page_w) / 2.0).round();
        let page_top = |i: usize| gap + i as f32 * (page_h + gap);

        let text_color = color(&theme.colors.text);
        let mut paras = Vec::with_capacity(self.doc.paragraphs.len());
        let mut page_i = 0;
        let mut y = mt;

        for para in &self.doc.paragraphs {
            let style = theme.style(&para.style);
            let size = style.size * pt;
            let fg = style.color.as_deref().map_or(text_color, color);
            let family = format!("{}:{}", theme.name, style.font);
            let format_for = |f: Fmt| {
                let face = fonts.face(&family, style.bold || f.bold, style.italic || f.italic);
                TextFormat {
                    font_id: FontId::new(size, face.family),
                    color: fg,
                    line_height: Some(size * style.line_height),
                    italics: face.synthetic_italic,
                    underline: if f.underline {
                        Stroke::new((size / 14.0).max(1.0), fg)
                    } else {
                        Stroke::NONE
                    },
                    ..Default::default()
                }
            };

            let mut job = LayoutJob::default();
            job.wrap.max_width = content_w;
            job.first_row_min_height = size * style.line_height;
            job.keep_trailing_whitespace = true;
            job.halign = match style.align {
                Align::Center => EAlign::Center,
                Align::Right => EAlign::Max,
                _ => EAlign::Min,
            };
            job.justify = style.align == Align::Justify;
            let indent = style.indent * inch;
            let runs = para.runs();
            if runs.is_empty() {
                job.append("", indent, format_for(Fmt::default()));
            }
            for (i, (text, f)) in runs.iter().enumerate() {
                job.append(text, if i == 0 { indent } else { 0.0 }, format_for(*f));
            }
            let galley = ui.painter().layout_job(job);
            let h = galley.size().y;

            let mut before = style.space_before * pt;
            if y > mt && y + before + h > page_h - mb {
                page_i += 1;
                y = mt;
            }
            if y <= mt {
                before = 0.0;
            }
            y += before;
            let anchor_x = match style.align {
                Align::Center => ml + content_w / 2.0,
                Align::Right => ml + content_w,
                _ => ml,
            };
            paras.push(ParaLayout {
                galley,
                origin: pos2(page_x + anchor_x, page_top(page_i) + y),
            });
            y += h + style.space_after * pt;
        }

        let pages: Vec<Rect> = (0..=page_i)
            .map(|i| Rect::from_min_size(pos2(page_x, page_top(i)), vec2(page_w, page_h)))
            .collect();
        let height = page_top(page_i + 1);
        Layout {
            paras,
            pages,
            size: vec2(canvas_w, height),
        }
    }

    fn ccursor(&self, ch: usize) -> CCursor {
        CCursor {
            index: CharIndex(ch),
            prefer_next_row: self.prefer_next_row,
        }
    }

    /// Caret rectangle in canvas space.
    fn caret_rect(&self, layout: &Layout, p: Pos) -> Rect {
        let pl = &layout.paras[p.para];
        pl.galley
            .pos_from_cursor(self.ccursor(p.ch))
            .translate(pl.origin.to_vec2())
    }

    fn pos_at(&self, layout: &Layout, point: Pos2) -> (Pos, bool) {
        let mut best = (0, f32::INFINITY);
        for (i, pl) in layout.paras.iter().enumerate() {
            let r = pl.galley.rect.translate(pl.origin.to_vec2());
            let d = if point.y < r.top() {
                r.top() - point.y
            } else if point.y > r.bottom() {
                point.y - r.bottom()
            } else {
                0.0
            };
            if d < best.1 {
                best = (i, d);
            }
        }
        let pl = &layout.paras[best.0];
        let c = pl.galley.cursor_from_pos(point - pl.origin);
        (Pos::new(best.0, c.index.0), c.prefer_next_row)
    }

    fn vertical(&self, layout: &Layout, up: bool) -> (Pos, f32) {
        let caret = self.caret_rect(layout, self.cursor);
        let x = self.goal_x.unwrap_or(caret.center().x);
        let pl = &layout.paras[self.cursor.para];
        let rows = &pl.galley.rows;
        let local_y = caret.center().y - pl.origin.y;
        let on_edge = if up {
            rows.first().is_none_or(|r| local_y < r.max_y())
        } else {
            rows.last().is_none_or(|r| local_y > r.min_y())
        };
        let target = if !on_edge {
            let y = if up { caret.top() - 1.0 } else { caret.bottom() + 1.0 };
            pos2(x, y)
        } else if up && self.cursor.para > 0 {
            let prev = &layout.paras[self.cursor.para - 1];
            pos2(x, prev.origin.y + prev.galley.rect.bottom() - 1.0)
        } else if !up && self.cursor.para + 1 < layout.paras.len() {
            let next = &layout.paras[self.cursor.para + 1];
            pos2(x, next.origin.y + next.galley.rect.top() + 1.0)
        } else if up {
            return (Pos::new(0, 0), x);
        } else {
            return (self.doc.end(), x);
        };
        (self.pos_at(layout, target).0, x)
    }

    fn row_edge(&self, layout: &Layout, end: bool) -> Pos {
        let pl = &layout.paras[self.cursor.para];
        let c = self.ccursor(self.cursor.ch);
        let c = if end {
            pl.galley.cursor_end_of_row(&c)
        } else {
            pl.galley.cursor_begin_of_row(&c)
        };
        Pos::new(self.cursor.para, c.index.0)
    }

    // ---- Frame ------------------------------------------------------------

    pub fn show(&mut self, ui: &mut Ui, theme: &Theme, fonts: &FontMap, page_text: PageText<'_>) {
        let bg = color(&theme.colors.background);
        ui.painter().rect_filled(ui.max_rect(), 0.0, bg);

        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let layout = self.layout(ui, theme, fonts, ui.available_width());
                self.page_count = layout.pages.len();
                let (rect, response) = ui.allocate_exact_size(layout.size, Sense::click_and_drag());
                let offset = rect.min.to_vec2();
                if response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                }

                self.handle_pointer(ui, &response, &layout, offset);
                self.handle_keys(ui, &layout, theme);
                // Edits change the layout, so lay out again before painting.
                let layout = self.layout(ui, theme, fonts, ui.available_width().max(rect.width()));
                let caret_y = self.caret_rect(&layout, self.cursor).center().y;
                self.caret_page = layout.pages.iter().position(|p| caret_y <= p.bottom()).unwrap_or(0) + 1;
                self.paint(ui, &layout, offset, theme);
                self.paint_margins(ui, &layout, offset, theme, page_text);
            });
    }

    fn handle_pointer(&mut self, ui: &Ui, response: &egui::Response, layout: &Layout, offset: Vec2) {
        let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) else {
            return;
        };
        let (p, prefer) = self.pos_at(layout, pointer - offset);
        let shift = ui.input(|i| i.modifiers.shift);
        if response.double_clicked() {
            self.anchor = self.doc.word_left(Pos::new(p.para, (p.ch + 1).min(self.doc.paragraphs[p.para].len())));
            self.cursor = self.doc.word_right(p);
            if self.anchor.para != self.cursor.para {
                self.anchor = Pos::new(p.para, 0);
            }
            self.pending = None;
        } else if response.drag_started() || (response.clicked() && !response.dragged()) {
            self.prefer_next_row = prefer;
            self.set_cursor(p, shift);
            self.goal_x = None;
            self.scroll_to_cursor = false;
        } else if response.dragged() {
            self.prefer_next_row = prefer;
            self.cursor = p;
        } else {
            return;
        }
        self.reset_caret(ui.input(|i| i.time));
    }

    fn handle_keys(&mut self, ui: &Ui, layout: &Layout, theme: &Theme) {
        let events = ui.input(|i| i.events.clone());
        let mut moved = false;
        for event in events {
            match event {
                egui::Event::Text(t) => self.insert_text(&t, theme, EditKind::Typing),
                egui::Event::Paste(t) => self.insert_text(&t, theme, EditKind::Other),
                egui::Event::Copy => {
                    if let Some((a, b)) = self.selection() {
                        ui.ctx().copy_text(self.doc.text(a, b));
                    }
                }
                egui::Event::Cut => {
                    if let Some((a, b)) = self.selection() {
                        ui.ctx().copy_text(self.doc.text(a, b));
                        self.delete_back(self.cursor);
                    }
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => moved |= self.key(key, modifiers, layout, theme),
                _ => continue,
            }
            self.reset_caret(ui.input(|i| i.time));
        }
        if !moved {
            return;
        }
        self.pending = None;
        self.last_edit = None;
        self.scroll_to_cursor = true;
    }

    /// Handles editing and movement keys. Returns true if the caret moved.
    fn key(&mut self, key: Key, m: Modifiers, layout: &Layout, theme: &Theme) -> bool {
        let p = self.cursor;
        let word = m.alt;
        let line = m.command;
        let mut target = None;
        let mut keep_goal = false;
        self.prefer_next_row = true;

        match key {
            Key::Enter => self.new_paragraph(theme),
            Key::Backspace => {
                let to = if line {
                    self.row_edge(layout, false)
                } else if word {
                    self.doc.word_left(p)
                } else if p.ch > 0 {
                    Pos::new(p.para, p.ch - 1)
                } else if p.para > 0 {
                    Pos::new(p.para - 1, self.doc.paragraphs[p.para - 1].len())
                } else {
                    p
                };
                self.delete_back(to);
            }
            Key::Delete => {
                let len = self.doc.paragraphs[p.para].len();
                let to = if word {
                    self.doc.word_right(p)
                } else if p.ch < len {
                    Pos::new(p.para, p.ch + 1)
                } else if p.para + 1 < self.doc.paragraphs.len() {
                    Pos::new(p.para + 1, 0)
                } else {
                    p
                };
                self.delete_forward(to);
            }
            Key::ArrowLeft => {
                target = Some(match (self.selection(), m.shift) {
                    (Some((a, _)), false) if !word && !line => a,
                    _ if line => self.row_edge(layout, false),
                    _ if word => self.doc.word_left(p),
                    _ if p.ch > 0 => Pos::new(p.para, p.ch - 1),
                    _ if p.para > 0 => Pos::new(p.para - 1, self.doc.paragraphs[p.para - 1].len()),
                    _ => p,
                });
            }
            Key::ArrowRight => {
                let len = self.doc.paragraphs[p.para].len();
                target = Some(match (self.selection(), m.shift) {
                    (Some((_, b)), false) if !word && !line => b,
                    _ if line => {
                        self.prefer_next_row = false;
                        self.row_edge(layout, true)
                    }
                    _ if word => self.doc.word_right(p),
                    _ if p.ch < len => Pos::new(p.para, p.ch + 1),
                    _ if p.para + 1 < self.doc.paragraphs.len() => Pos::new(p.para + 1, 0),
                    _ => p,
                });
            }
            Key::ArrowUp | Key::ArrowDown => {
                let up = key == Key::ArrowUp;
                target = Some(if line {
                    if up { Pos::default() } else { self.doc.end() }
                } else {
                    let (t, x) = self.vertical(layout, up);
                    self.goal_x = Some(x);
                    keep_goal = true;
                    t
                });
            }
            Key::Home => target = Some(self.row_edge(layout, false)),
            Key::End => {
                self.prefer_next_row = false;
                target = Some(self.row_edge(layout, true));
            }
            _ => return false,
        }

        if !keep_goal {
            self.goal_x = None;
        }
        match target {
            Some(t) => {
                let prefer = self.prefer_next_row;
                self.set_cursor(t, m.shift);
                self.prefer_next_row = prefer;
                true
            }
            None => false,
        }
    }

    /// Draws each visible page's header and footer in its top and bottom margins.
    fn paint_margins(&self, ui: &Ui, layout: &Layout, offset: Vec2, theme: &Theme, page_text: PageText<'_>) {
        if theme.page.header.is_none() && theme.page.footer.is_none() {
            return;
        }
        let inch = DPI * self.zoom;
        let font = FontId::proportional(9.0 * inch / 72.0);
        let fg = color(&theme.colors.text).gamma_multiply(0.6);
        let pages = layout.pages.len();
        for (i, page) in layout.pages.iter().enumerate() {
            let r = page.translate(offset);
            if !r.intersects(ui.clip_rect()) {
                continue;
            }
            let (header, footer) = page_text(i + 1, pages);
            let painter = ui.painter();
            let top = r.top() + theme.page.margin_top * inch / 2.0;
            let bottom = r.bottom() - theme.page.margin_bottom * inch / 2.0;
            painter.text(pos2(r.center().x, top), egui::Align2::CENTER_CENTER, header, font.clone(), fg);
            painter.text(pos2(r.center().x, bottom), egui::Align2::CENTER_CENTER, footer, font.clone(), fg);
        }
    }

    fn paint(&mut self, ui: &mut Ui, layout: &Layout, offset: Vec2, theme: &Theme) {
        let painter = ui.painter();
        let colors = &theme.colors;
        let page_fill = color(&colors.page);
        let border = color(&colors.page_border);
        let visible = ui.clip_rect();

        for page in &layout.pages {
            let r = page.translate(offset);
            if !r.intersects(visible) {
                continue;
            }
            painter.rect_filled(r.translate(vec2(0.0, 3.0)).expand(1.0), CornerRadius::same(2), Color32::from_black_alpha(40));
            painter.rect_filled(r, CornerRadius::ZERO, page_fill);
            painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.0, border), StrokeKind::Outside);
        }

        if let Some((a, b)) = self.selection() {
            let sel = color(&colors.selection);
            for i in a.para..=b.para {
                let pl = &layout.paras[i];
                let start = if i == a.para { a.ch } else { 0 };
                let end = if i == b.para { b.ch } else { self.doc.paragraphs[i].len() };
                let continues = i < b.para;
                let mut idx = 0;
                for row in &pl.galley.rows {
                    let n = row.char_count_excluding_newline().0;
                    let (row_start, row_end) = (idx, idx + n);
                    idx += row.char_count_including_newline().0;
                    let (s, e) = (start.max(row_start), end.min(row_end));
                    let reaches_end = continues && end >= row_end;
                    if s > e || (s == e && !reaches_end) {
                        continue;
                    }
                    let x0 = pl.galley.pos_from_cursor(CCursor { index: CharIndex(s), prefer_next_row: true }).min.x;
                    let mut x1 = pl.galley.pos_from_cursor(CCursor { index: CharIndex(e), prefer_next_row: false }).min.x;
                    if reaches_end {
                        x1 += 5.0 * self.zoom;
                    }
                    let r = Rect::from_x_y_ranges(x0..=x1, row.min_y()..=row.max_y());
                    painter.rect_filled(r.translate(pl.origin.to_vec2() + offset), 0.0, sel);
                }
            }
        }

        for pl in &layout.paras {
            let r = pl.galley.rect.translate(pl.origin.to_vec2() + offset);
            if r.intersects(visible) {
                painter.galley(pl.origin + offset, pl.galley.clone(), Color32::BLACK);
            }
        }

        let caret = self.caret_rect(layout, self.cursor).translate(offset);
        let t = ui.input(|i| i.time) - self.blink_start;
        if ui.ctx().input(|i| i.focused) && t % 1.0 < 0.6 {
            let w = (1.5 * self.zoom).max(1.0);
            painter.rect_filled(
                Rect::from_x_y_ranges(caret.min.x..=caret.min.x + w, caret.y_range()),
                0.0,
                color(&colors.cursor),
            );
        }
        ui.ctx().request_repaint_after(Duration::from_millis(300));

        if self.scroll_to_cursor {
            ui.scroll_to_rect(caret.expand(40.0), None);
            self.scroll_to_cursor = false;
        }
    }
}
