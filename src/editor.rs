//! Page layout, painting and text editing for a [`Document`].
//!
//! egui only ships a plain-text editor, so this module does its own rich-text
//! editing on top of egui's text layout (`Galley`): each paragraph is laid out
//! separately, then placed onto pages.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{
    self, Align as EAlign, Color32, CornerRadius, FontId, Galley, Key, Modifiers, Pos2, Rect,
    Sense, Stroke, StrokeKind, Ui, Vec2, pos2, text::CCursor, text::CharIndex, text::LayoutJob,
    text::TextFormat, vec2,
};

use crate::doc::{Document, Fmt, Pos};
use crate::pdf;
use crate::spell::Speller;
use crate::theme::{Align, FontMap, Theme, color};

/// Screen points per inch at 100% zoom.
const DPI: f32 = 96.0;

struct ParaLayout {
    galley: Arc<Galley>,
    /// Where the paragraph's rows sit on the canvas. Usually one piece; a
    /// paragraph that runs past the bottom of a page continues as a second
    /// piece at the top of the next.
    pieces: Vec<Piece>,
}

struct Piece {
    rows: Range<usize>,
    /// Galley anchor in canvas space for these rows.
    origin: Pos2,
}

impl ParaLayout {
    /// Index of the galley row at a galley-local y.
    fn row_at(&self, local_y: f32) -> usize {
        let rows = &self.galley.rows;
        rows.iter().position(|r| local_y < r.max_y()).unwrap_or(rows.len().saturating_sub(1))
    }

    fn piece_for_row(&self, row: usize) -> &Piece {
        self.pieces
            .iter()
            .find(|p| p.rows.contains(&row))
            .unwrap_or(&self.pieces[self.pieces.len() - 1])
    }

    /// A galley-local rect moved to wherever its row is drawn.
    fn to_canvas(&self, local: Rect) -> Rect {
        let piece = self.piece_for_row(self.row_at(local.center().y));
        local.translate(piece.origin.to_vec2())
    }

    /// Galley-local y range covered by a piece.
    fn local_y_range(&self, piece: &Piece) -> (f32, f32) {
        let rows = &self.galley.rows;
        match (rows.get(piece.rows.start), piece.rows.end.checked_sub(1).and_then(|i| rows.get(i))) {
            (Some(first), Some(last)) => (first.min_y(), last.max_y()),
            _ => (self.galley.rect.top(), self.galley.rect.bottom()),
        }
    }

    /// Canvas rect of a piece (full galley width).
    fn piece_rect(&self, piece: &Piece) -> Rect {
        let (top, bottom) = self.local_y_range(piece);
        Rect::from_x_y_ranges(self.galley.rect.x_range(), top..=bottom).translate(piece.origin.to_vec2())
    }
}

/// Rows, page, and the page-relative y of the galley's local 0, for each piece.
type Placed = Vec<(Range<usize>, usize, f32)>;

/// Splits rows (galley-local `(min_y, max_y)`) across pages.
///
/// `y` is where the paragraph starts, measured from the page top. Returns,
/// for each piece, its rows, page, and the page-relative y of the galley's
/// local 0, plus the page and y where the paragraph ends.
fn paginate(
    rows: &[(f32, f32)],
    mut page: usize,
    y: f32,
    margin_top: f32,
    bottom: f32,
) -> (Placed, usize, f32) {
    if rows.is_empty() {
        return (vec![(0..0, page, y)], page, y);
    }
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut offset = y - rows[0].0;
    for (i, &(top, end)) in rows.iter().enumerate() {
        // A row that would cross the bottom margin starts the next page,
        // unless it's already the first row on this page.
        if i > start && offset + end > bottom {
            pieces.push((start..i, page, offset));
            page += 1;
            start = i;
            offset = margin_top - top;
        }
    }
    pieces.push((start..rows.len(), page, offset));
    let end_y = offset + rows[rows.len() - 1].1;
    (pieces, page, end_y)
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
    /// Ranges to highlight, e.g. find matches.
    pub highlights: Vec<(Pos, Pos)>,
    /// False while another widget (like the find bar) has the keyboard.
    pub keyboard: bool,
    pub smart_quotes: bool,
    pub smart_dashes: bool,
    /// The misspelled word that was right-clicked, with its suggestions.
    spell_target: Option<SpellTarget>,
}

#[derive(Clone)]
struct SpellTarget {
    para: usize,
    range: Range<usize>,
    word: String,
    suggestions: Vec<String>,
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
            highlights: Vec::new(),
            keyboard: true,
            smart_quotes: true,
            smart_dashes: true,
            spell_target: None,
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

    /// Turns the character just typed into its typographic form: curly
    /// quotes, em dashes, ellipses. It's a separate undo step, so Cmd+Z
    /// right after gives back the plain character.
    fn smarten(&mut self, typed: char) {
        let Pos { para, ch } = self.cursor;
        let chars = &self.doc.paragraphs[para].chars;
        if ch == 0 || chars.get(ch - 1) != Some(&typed) {
            return;
        }
        let before = |n: usize| ch.checked_sub(n).and_then(|i| chars.get(i)).copied();
        let opens = |c: Option<char>| c.is_none_or(|c| c.is_whitespace() || "([{“‘—–/".contains(c));
        let (remove, with) = match typed {
            '"' if self.smart_quotes => (1, if opens(before(2)) { '“' } else { '”' }),
            '\'' if self.smart_quotes => (1, if opens(before(2)) { '‘' } else { '’' }),
            '-' if self.smart_dashes && before(2) == Some('-') => (2, '—'),
            '.' if self.smart_dashes && before(2) == Some('.') && before(3) == Some('.') => (3, '…'),
            _ => return,
        };
        self.checkpoint(EditKind::Other);
        let start = Pos::new(para, ch - remove);
        let fmt = self.doc.paragraphs[para].fmts[ch - remove];
        self.doc.delete(start, self.cursor);
        self.cursor = self.doc.insert(start, &with.to_string(), fmt, "body");
        self.anchor = self.cursor;
        // The next typed character starts a fresh undo group.
        self.last_edit = None;
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

    pub fn selection_range(&self) -> Option<(Pos, Pos)> {
        self.selection()
    }

    /// Selects a range and scrolls it into view.
    pub fn select(&mut self, a: Pos, b: Pos) {
        self.set_cursor(a, false);
        self.cursor = self.doc.clamp(b);
        self.goal_x = None;
    }

    /// Replaces each range with `text` as one undo step. Ranges must be in
    /// document order and not overlap.
    pub fn replace_ranges(&mut self, ranges: &[(Pos, Pos)], text: &str, theme: &Theme) {
        if ranges.is_empty() {
            return;
        }
        self.checkpoint(EditKind::Other);
        let mut last = self.cursor;
        for &(a, b) in ranges.iter().rev() {
            let fmt = self.doc.fmt_at(Pos::new(a.para, a.ch + 1).min(b));
            let next = theme.next_style(&self.doc.paragraphs[a.para].style);
            self.doc.delete(a, b);
            last = self.doc.insert(a, text, fmt, &next);
        }
        // Leave the caret after the first replacement when there's just one.
        if ranges.len() == 1 {
            self.set_cursor(last, false);
        }
        self.cursor = self.doc.clamp(self.cursor);
        self.anchor = self.cursor;
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

    fn layout(&self, ui: &Ui, theme: &Theme, fonts: &FontMap, min_width: f32, zoom: f32) -> Layout {
        let inch = DPI * zoom;
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
            let rows: Vec<(f32, f32)> = galley.rows.iter().map(|r| (r.min_y(), r.max_y())).collect();
            let first_row = rows.first().map_or(galley.size().y, |r| r.1 - r.0);

            let mut before = style.space_before * pt;
            // Start on a new page if not even the first line fits.
            if y > mt && y + before + first_row > page_h - mb {
                page_i += 1;
                y = mt;
            }
            if y <= mt {
                before = 0.0;
            }
            y += before;
            let anchor_x = page_x
                + match style.align {
                    Align::Center => ml + content_w / 2.0,
                    Align::Right => ml + content_w,
                    _ => ml,
                };
            let (placed, end_page, end_y) = paginate(&rows, page_i, y, mt, page_h - mb);
            let pieces = placed
                .into_iter()
                .map(|(rows, page, offset)| Piece {
                    rows,
                    origin: pos2(anchor_x, page_top(page) + offset),
                })
                .collect();
            paras.push(ParaLayout { galley, pieces });
            page_i = end_page;
            y = end_y + style.space_after * pt;
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
        pl.to_canvas(pl.galley.pos_from_cursor(self.ccursor(p.ch)))
    }

    fn pos_at(&self, layout: &Layout, point: Pos2) -> (Pos, bool) {
        let mut best = (0, 0, f32::INFINITY);
        for (i, pl) in layout.paras.iter().enumerate() {
            for (k, piece) in pl.pieces.iter().enumerate() {
                let r = pl.piece_rect(piece);
                let d = if point.y < r.top() {
                    r.top() - point.y
                } else if point.y > r.bottom() {
                    point.y - r.bottom()
                } else {
                    0.0
                };
                if d < best.2 {
                    best = (i, k, d);
                }
            }
        }
        let pl = &layout.paras[best.0];
        let piece = &pl.pieces[best.1];
        // Keep the lookup inside this piece's rows; the galley's other rows
        // are drawn on another page.
        let (top, bottom) = pl.local_y_range(piece);
        let mut local = point - piece.origin;
        local.y = local.y.clamp(top + 0.5, (bottom - 0.5).max(top + 0.5));
        let c = pl.galley.cursor_from_pos(local);
        (Pos::new(best.0, c.index.0), c.prefer_next_row)
    }

    /// The character nearest canvas x on one row of a paragraph.
    fn pos_in_row(&self, layout: &Layout, para: usize, row: usize, x: f32) -> Pos {
        let pl = &layout.paras[para];
        let Some(r) = pl.galley.rows.get(row) else {
            return Pos::new(para, 0);
        };
        let piece = pl.piece_for_row(row);
        let local = vec2(x - piece.origin.x, (r.min_y() + r.max_y()) / 2.0);
        Pos::new(para, pl.galley.cursor_from_pos(local).index.0)
    }

    fn vertical(&self, layout: &Layout, up: bool) -> (Pos, f32) {
        let caret = self.caret_rect(layout, self.cursor);
        let x = self.goal_x.unwrap_or(caret.center().x);
        let para = self.cursor.para;
        let pl = &layout.paras[para];
        let local = pl.galley.pos_from_cursor(self.ccursor(self.cursor.ch));
        let row = pl.row_at(local.center().y);
        let rows = pl.galley.rows.len();

        let target = if up && row > 0 {
            self.pos_in_row(layout, para, row - 1, x)
        } else if !up && row + 1 < rows {
            self.pos_in_row(layout, para, row + 1, x)
        } else if up && para > 0 {
            let last = layout.paras[para - 1].galley.rows.len().saturating_sub(1);
            self.pos_in_row(layout, para - 1, last, x)
        } else if !up && para + 1 < layout.paras.len() {
            self.pos_in_row(layout, para + 1, 0, x)
        } else if up {
            Pos::new(0, 0)
        } else {
            self.doc.end()
        };
        (target, x)
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

    // ---- Printing ---------------------------------------------------------

    /// The document as it would print: the same layout as the screen at 100%,
    /// converted to PDF points, with every word placed where egui put it.
    pub fn print_model(&self, ui: &Ui, theme: &Theme, fonts: &FontMap, page_text: PageText<'_>) -> pdf::Model {
        let layout = self.layout(ui, theme, fonts, 0.0, 1.0);
        let k = 72.0 / DPI;
        let mut pages: Vec<pdf::Page> = layout.pages.iter().map(|_| pdf::Page::default()).collect();
        let page_of = |y: f32| layout.pages.iter().position(|p| y < p.bottom()).unwrap_or(layout.pages.len() - 1);
        let text_color = color(&theme.colors.text);

        for (i, pl) in layout.paras.iter().enumerate() {
            let para = &self.doc.paragraphs[i];
            let style = theme.style(&para.style);
            let family = format!("{}:{}", theme.name, style.font);
            let fg = style.color.as_deref().map_or(text_color, color);
            let mut ch = 0; // character index where the row starts
            for (r, row) in pl.galley.rows.iter().enumerate() {
                let base = pl.piece_for_row(r).origin + row.pos.to_vec2();
                let page_i = page_of(base.y);
                let page_min = layout.pages[page_i].min.to_vec2();
                let page = &mut pages[page_i];
                let mut word: Option<(pdf::Text, Fmt)> = None;

                for (g, glyph) in row.glyphs.iter().enumerate() {
                    let fmt = para.fmts.get(ch + g).copied().unwrap_or_default();
                    let at = (base + glyph.pos.to_vec2() - page_min) * k;
                    if fmt.underline {
                        let thickness = (style.size / 14.0).max(0.5);
                        page.rects.push(pdf::Block {
                            x: at.x,
                            y: at.y + style.size * 0.12,
                            w: glyph.advance_width * k,
                            h: thickness,
                            color: fg,
                        });
                    }
                    if glyph.chr.is_whitespace() {
                        page.texts.extend(word.take().map(|w| w.0));
                        continue;
                    }
                    match &mut word {
                        Some((w, f)) if *f == fmt => w.text.push(glyph.chr),
                        _ => {
                            page.texts.extend(word.take().map(|w| w.0));
                            let (font, slant) = fonts.file(&family, style.bold || fmt.bold, style.italic || fmt.italic);
                            word = Some((
                                pdf::Text {
                                    x: at.x,
                                    y: at.y,
                                    text: glyph.chr.to_string(),
                                    font,
                                    size: style.size,
                                    color: fg,
                                    slant,
                                },
                                fmt,
                            ));
                        }
                    }
                }
                page.texts.extend(word.take().map(|w| w.0));
                ch += row.char_count_including_newline().0;
            }
        }

        // Headers and footers, centered in the top and bottom margins.
        let (w, h) = (theme.page.width * 72.0, theme.page.height * 72.0);
        let size = 9.0;
        let faint = text_color.gamma_multiply(0.6);
        let count = pages.len();
        for (i, page) in pages.iter_mut().enumerate() {
            let (header, footer) = page_text(i + 1, count);
            for (text, mid) in [(header, theme.page.margin_top * 36.0), (footer, h - theme.page.margin_bottom * 36.0)] {
                if text.is_empty() {
                    continue;
                }
                let width = ui.painter().layout_no_wrap(text.clone(), FontId::proportional(size / k), faint).size().x * k;
                page.texts.push(pdf::Text {
                    x: (w - width) / 2.0,
                    y: mid + size * 0.35,
                    text,
                    font: None,
                    size,
                    color: faint,
                    slant: false,
                });
            }
        }

        pdf::Model {
            width: w,
            height: h,
            background: theme.page.print_background.then(|| color(&theme.colors.page)),
            pages,
        }
    }

    // ---- Frame ------------------------------------------------------------

    pub fn show(
        &mut self,
        ui: &mut Ui,
        theme: &Theme,
        fonts: &FontMap,
        page_text: PageText<'_>,
        mut speller: Option<&mut Speller>,
    ) {
        let bg = color(&theme.colors.background);
        ui.painter().rect_filled(ui.max_rect(), 0.0, bg);

        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let layout = self.layout(ui, theme, fonts, ui.available_width(), self.zoom);
                self.page_count = layout.pages.len();
                let (rect, response) = ui.allocate_exact_size(layout.size, Sense::click_and_drag());
                let offset = rect.min.to_vec2();
                if response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                }

                self.handle_pointer(ui, &response, &layout, offset);
                if let Some(speller) = speller.as_deref_mut() {
                    self.spelling_menu(ui, &response, &layout, offset, theme, speller);
                }
                if self.keyboard {
                    self.handle_keys(ui, &layout, theme);
                }
                // Edits change the layout, so lay out again before painting.
                let layout = self.layout(ui, theme, fonts, ui.available_width().max(rect.width()), self.zoom);
                let caret_y = self.caret_rect(&layout, self.cursor).center().y;
                self.caret_page = layout.pages.iter().position(|p| caret_y <= p.bottom()).unwrap_or(0) + 1;
                self.paint(ui, &layout, offset, theme);
                if let Some(speller) = speller {
                    let words = self.misspelled(speller);
                    self.paint_spelling(ui, &layout, offset, &words);
                }
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
                egui::Event::Text(t) => {
                    self.insert_text(&t, theme, EditKind::Typing);
                    let mut chars = t.chars();
                    if let (Some(c), None) = (chars.next(), chars.next()) {
                        self.smarten(c);
                    }
                }
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

        let highlight = color(colors.find.as_deref().unwrap_or("#ffcc0055"));
        for &(a, b) in &self.highlights {
            self.paint_range(painter, layout, offset, a, b, highlight);
        }

        if let Some((a, b)) = self.selection() {
            let sel = color(&colors.selection);
            self.paint_range(painter, layout, offset, a, b, sel);
        }
        self.paint_text_and_caret(ui, layout, offset, theme);
    }

    /// Fills the area behind a range of text, row by row.
    fn paint_range(&self, painter: &egui::Painter, layout: &Layout, offset: Vec2, a: Pos, b: Pos, fill: Color32) {
        for r in self.range_rects(layout, a, b) {
            painter.rect_filled(r.translate(offset), 0.0, fill);
        }
    }

    /// One canvas rect per row a range covers.
    fn range_rects(&self, layout: &Layout, a: Pos, b: Pos) -> Vec<Rect> {
        let mut rects = Vec::new();
        {
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
                    rects.push(pl.to_canvas(r));
                }
            }
        }
        rects
    }

    /// Red wavy lines under misspelled words.
    fn paint_spelling(&self, ui: &Ui, layout: &Layout, offset: Vec2, words: &[(usize, Range<usize>)]) {
        let red = Color32::from_rgb(230, 60, 60);
        let step = 2.0 * self.zoom.max(0.75);
        let visible = ui.clip_rect();
        for (para, range) in words {
            for r in self.range_rects(layout, Pos::new(*para, range.start), Pos::new(*para, range.end)) {
                let r = r.translate(offset);
                if !r.intersects(visible) {
                    continue;
                }
                let y = r.bottom() - step;
                let points: Vec<Pos2> = (0..=((r.width() / step) as usize))
                    .map(|i| pos2(r.left() + i as f32 * step, if i % 2 == 0 { y } else { y + step * 0.7 }))
                    .collect();
                ui.painter().add(egui::Shape::line(points, Stroke::new(self.zoom.max(0.75), red)));
            }
        }
    }

    /// Misspelled words to underline, leaving alone the word being typed.
    fn misspelled(&self, speller: &mut Speller) -> Vec<(usize, Range<usize>)> {
        let mut out = Vec::new();
        for (i, para) in self.doc.paragraphs.iter().enumerate() {
            for r in speller.misspellings(&para.chars) {
                let typing_here = i == self.cursor.para
                    && self.selection().is_none()
                    && r.start <= self.cursor.ch
                    && self.cursor.ch <= r.end
                    && self.last_edit == Some(EditKind::Typing);
                if !typing_here {
                    out.push((i, r.clone()));
                }
            }
        }
        out
    }

    /// The right-click menu for a misspelled word.
    fn spelling_menu(&mut self, ui: &Ui, response: &egui::Response, layout: &Layout, offset: Vec2, theme: &Theme, speller: &mut Speller) {
        if response.secondary_clicked() {
            self.spell_target = ui.input(|i| i.pointer.interact_pos()).and_then(|at| {
                let (p, _) = self.pos_at(layout, at - offset);
                let chars = &self.doc.paragraphs[p.para].chars;
                let range = speller.misspellings(chars).iter().find(|r| r.start <= p.ch && p.ch <= r.end)?.clone();
                Some(SpellTarget {
                    para: p.para,
                    word: chars[range.clone()].iter().collect(),
                    suggestions: speller.suggestions(chars, range.clone()),
                    range,
                })
            });
        }
        let Some(target) = self.spell_target.clone() else { return };
        let shown = response.context_menu(|ui| {
            if target.suggestions.is_empty() {
                ui.label(egui::RichText::new("No suggestions").weak());
            }
            for s in &target.suggestions {
                if ui.button(s).clicked() {
                    let range = (Pos::new(target.para, target.range.start), Pos::new(target.para, target.range.end));
                    self.replace_ranges(&[range], s, theme);
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Learn Spelling").clicked() {
                speller.learn(&target.word);
                ui.close();
            }
            if ui.button("Ignore Spelling").clicked() {
                speller.ignore(&target.word);
                ui.close();
            }
        });
        if shown.is_none() && !response.secondary_clicked() {
            self.spell_target = None;
        }
    }

    fn paint_text_and_caret(&mut self, ui: &mut Ui, layout: &Layout, offset: Vec2, theme: &Theme) {
        let painter = ui.painter();
        let colors = &theme.colors;
        let visible = ui.clip_rect();
        for pl in &layout.paras {
            for piece in &pl.pieces {
                let r = pl.piece_rect(piece).translate(offset);
                if !r.intersects(visible) {
                    continue;
                }
                // A split paragraph is drawn once per piece, each clipped to its own rows.
                let clip = if pl.pieces.len() == 1 {
                    visible
                } else {
                    Rect::from_x_y_ranges(visible.x_range(), r.y_range()).intersect(visible)
                };
                painter.with_clip_rect(clip).galley(piece.origin + offset, pl.galley.clone(), Color32::BLACK);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Ten 20pt rows starting 600pt down a page whose text area ends at 700pt.
    #[test]
    fn long_paragraph_continues_on_next_page() {
        let rows: Vec<(f32, f32)> = (0..10).map(|i| (i as f32 * 20.0, i as f32 * 20.0 + 20.0)).collect();
        let (pieces, end_page, end_y) = paginate(&rows, 0, 600.0, 72.0, 700.0);

        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0], (0..5, 0, 600.0), "rows 0-4 fit (the last ends at 700)");
        // Row 5 is at local y 100 and lands at the top margin of page 1.
        assert_eq!(pieces[1], (5..10, 1, -28.0));
        assert_eq!((end_page, end_y), (1, 72.0 + 100.0));
    }

    #[test]
    fn short_paragraph_and_empty_paragraph_stay_whole() {
        let (pieces, page, y) = paginate(&[(0.0, 20.0)], 2, 100.0, 72.0, 700.0);
        assert_eq!(pieces, vec![(0..1, 2, 100.0)]);
        assert_eq!((page, y), (2, 120.0));
        let (pieces, ..) = paginate(&[], 0, 100.0, 72.0, 700.0);
        assert_eq!(pieces.len(), 1);
    }

    fn typed(text: &str) -> Editor {
        let theme: Theme = toml::from_str(crate::theme::DEFAULT_THEME).unwrap();
        let mut ed = Editor::new(Document::default());
        for c in text.chars() {
            ed.insert_text(&c.to_string(), &theme, EditKind::Typing);
            ed.smarten(c);
        }
        ed
    }

    fn plain(ed: &Editor) -> String {
        ed.doc.text(Pos::default(), ed.doc.end())
    }

    #[test]
    fn smart_quotes_and_dashes() {
        assert_eq!(plain(&typed("\"Hi,\" she said. It's 'fine'")), "“Hi,” she said. It’s ‘fine’");
        assert_eq!(plain(&typed("(\"x\")")), "(“x”)");
        assert_eq!(plain(&typed("wait--what...")), "wait—what…");
        assert_eq!(plain(&typed("a - b")), "a - b", "a single hyphen stays");

        let mut ed = typed("rock--");
        ed.undo();
        assert_eq!(plain(&ed), "rock--", "undo right after gives back the plain characters");

        let mut ed = typed("x");
        ed.smart_quotes = false;
        let theme: Theme = toml::from_str(crate::theme::DEFAULT_THEME).unwrap();
        ed.insert_text("\"", &theme, EditKind::Typing);
        ed.smarten('"');
        assert_eq!(plain(&ed), "x\"", "can be turned off");
    }

    #[test]
    fn replace_all_is_one_undo_step() {
        let theme: Theme = toml::from_str(crate::theme::DEFAULT_THEME).unwrap();
        let mut doc = Document::default();
        doc.insert(Pos::default(), "cat and cat\nlast cat", Fmt::default(), "body");
        let mut ed = Editor::new(doc);
        let matches = ed.doc.find_all("cat", false, true);
        ed.replace_ranges(&matches, "dog", &theme);
        assert_eq!(ed.doc.text(Pos::default(), ed.doc.end()), "dog and dog\nlast dog");
        ed.undo();
        assert_eq!(ed.doc.text(Pos::default(), ed.doc.end()), "cat and cat\nlast cat");
    }

    /// Lays out a real document headlessly with the Paper theme and checks the
    /// print model: several pages, every word present, headings in bold Georgia.
    #[test]
    fn print_model_matches_the_document() {
        let ctx = egui::Context::default();
        let theme: Theme = toml::from_str(crate::theme::DEFAULT_THEME).unwrap();
        let fonts = crate::theme::install_fonts(&ctx, std::slice::from_ref(&theme));

        let mut doc = Document::default();
        let long = "The quick brown fox jumps over the lazy dog. ".repeat(60);
        let mut p = doc.insert(Pos::default(), "A Heading", Fmt::default(), "body");
        for _ in 0..8 {
            p = doc.insert(p, &format!("\n{long}"), Fmt::default(), "body");
        }
        doc.paragraphs[0].style = "h1".into();
        let editor = Editor::new(doc);

        let mut model = None;
        // Fonts take effect on the second pass.
        for _ in 0..2 {
            let mut out = ctx.run_ui(Default::default(), |ui| {
                model = Some(editor.print_model(ui, &theme, &fonts, &|p, n| (String::new(), format!("{p} / {n}"))));
            });
            out.textures_delta.clear(); // no renderer in tests
        }
        let model = model.unwrap();

        assert!(model.pages.len() >= 3, "8 long paragraphs span pages: {}", model.pages.len());
        assert_eq!((model.width, model.height), (612.0, 792.0));
        let words: Vec<&str> = model.pages.iter().flat_map(|p| &p.texts).map(|t| t.text.as_str()).collect();
        assert_eq!(words.iter().filter(|w| **w == "fox").count(), 8 * 60, "every word is printed once");
        let heading = model.pages[0].texts.iter().find(|t| t.text == "Heading").unwrap();
        assert!(heading.font.as_ref().unwrap().ends_with("Georgia Bold.ttf"));
        assert_eq!(heading.size, 20.0);
        assert!(model.pages[1].texts.iter().any(|t| t.text == "2 / 3" || t.text.starts_with("2 /")), "footer");
        for page in &model.pages {
            for t in &page.texts {
                assert!(t.y > 0.0 && t.y < 792.0 && t.x >= 0.0 && t.x < 612.0, "{} at {},{}", t.text, t.x, t.y);
            }
        }
        assert!(!crate::pdf::render(&model, "t").unwrap().is_empty());
    }

    /// `FOLIO_PDF_OUT=/path/out.pdf cargo test export_sample -- --ignored`
    /// writes examples/welcome.folio as a PDF, for checking by eye.
    #[test]
    #[ignore]
    fn export_sample() {
        let ctx = egui::Context::default();
        let theme: Theme = toml::from_str(crate::theme::DEFAULT_THEME).unwrap();
        let fonts = crate::theme::install_fonts(&ctx, std::slice::from_ref(&theme));
        let doc = Document::from_json(&std::fs::read_to_string("examples/welcome.folio").unwrap()).unwrap();
        let editor = Editor::new(doc);
        let mut model = None;
        for _ in 0..2 {
            let mut out = ctx.run_ui(Default::default(), |ui| {
                model = Some(editor.print_model(ui, &theme, &fonts, &|p, n| (String::new(), format!("{p} / {n}"))));
            });
            out.textures_delta.clear();
        }
        let path = std::env::var("FOLIO_PDF_OUT").unwrap_or_else(|_| "welcome.pdf".into());
        std::fs::write(path, crate::pdf::render(&model.unwrap(), "Welcome").unwrap()).unwrap();
    }

    #[test]
    fn a_row_taller_than_the_page_still_moves_forward() {
        let rows = [(0.0, 900.0), (900.0, 920.0)];
        let (pieces, page, _) = paginate(&rows, 0, 72.0, 72.0, 700.0);
        assert_eq!(pieces.len(), 2, "the giant row stays alone; the next one breaks");
        assert_eq!(page, 1);
    }
}
