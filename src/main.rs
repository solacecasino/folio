//! Folio: a document editor that starts bare and gets out of the way.
//! Everything about how documents look comes from theme files.

mod components;
mod doc;
mod editor;
mod menu;
mod theme;

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Align2, FontId, Key, Modifiers, Sense, SystemTheme, ViewportCommand};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult};
use serde_json::{Value, json};

use components::{Call, Host};
use doc::{Document, Fmt, Pos};
use editor::Editor;
use menu::{Command, ComponentMenu, MenuBar};
use theme::{FontMap, Theme, color};

const HELP: &str = "\
Cmd+N  new            Cmd+O  open
Cmd+S  save           Cmd+Shift+S  save as
Cmd+B / I / U  bold, italic, underline
Cmd+0-9  paragraph styles (set in the theme)
Cmd+T  next theme     Cmd+= / Cmd+-  zoom
Cmd+Z  undo           Cmd+Shift+Z  redo
Cmd+.  status line    Cmd+/  this help";

pub const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

/// Height of the strip drawn where the macOS titlebar would be.
const TITLEBAR: f32 = 28.0;

struct Folio {
    editor: Editor,
    path: Option<PathBuf>,
    themes: Vec<Theme>,
    theme: usize,
    fonts: FontMap,
    show_status: bool,
    show_help: bool,
    title: String,
    error: Option<String>,
    menu: MenuBar,
    components: Host,
    /// A short message from a component, and when it arrived.
    notice: Option<(String, f64)>,
    /// Last document revision components were told about, and when.
    sent_revision: (u64, f64),
    /// Background color last used to pick a light or dark window appearance.
    appearance_for: Option<egui::Color32>,
}

impl Folio {
    fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>, icon: &egui::IconData) -> Self {
        let themes = theme::load_themes();
        let fonts = theme::install_fonts(&cc.egui_ctx, &themes);
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);

        let found = components::discover();
        let menus: Vec<ComponentMenu> = found
            .iter()
            .map(|c| ComponentMenu {
                name: c.manifest.name.clone(),
                items: c.manifest.commands.iter().map(|cmd| (cmd.label.clone(), cmd.keys.clone())).collect(),
            })
            .collect();
        let ctx = cc.egui_ctx.clone();
        let host = Host::new(
            found,
            Arc::new(move || ctx.request_repaint()),
            Box::new(components::approve_with_dialog),
        );

        let mut app = Self {
            editor: Editor::new(Document::default()),
            path: None,
            themes,
            theme: 0,
            fonts,
            show_status: true,
            show_help: false,
            title: String::new(),
            error: None,
            menu: MenuBar::install(&cc.egui_ctx, icon, &menus),
            components: host,
            notice: None,
            sent_revision: (0, 0.0),
            appearance_for: None,
        };
        if let Some(p) = path {
            app.open_path(p);
        }
        app
    }

    fn theme(&self) -> &Theme {
        &self.themes[self.theme]
    }

    fn open_path(&mut self, path: PathBuf) {
        let result = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|s| Document::from_json(&s));
        match result {
            Ok(doc) => {
                if let Some(i) = doc
                    .theme
                    .as_ref()
                    .and_then(|name| self.themes.iter().position(|t| &t.name == name))
                {
                    self.theme = i;
                }
                self.editor = Editor::new(doc);
                self.components.emit("opened", json!({ "path": path }));
                self.path = Some(path);
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Couldn't open {}: {e}", path.display())),
        }
    }

    fn save(&mut self, choose: bool) -> bool {
        let path = match (&self.path, choose) {
            (Some(p), false) => p.clone(),
            _ => match FileDialog::new()
                .add_filter("Folio document", &["folio"])
                .set_file_name("Untitled.folio")
                .save_file()
            {
                Some(p) => p,
                None => return false,
            },
        };
        self.editor.doc.theme = Some(self.theme().name.clone());
        match std::fs::write(&path, self.editor.doc.to_json()) {
            Ok(()) => {
                self.components.emit("saved", json!({ "path": path }));
                self.path = Some(path);
                self.editor.dirty = false;
                self.error = None;
                true
            }
            Err(e) => {
                self.error = Some(format!("Couldn't save: {e}"));
                false
            }
        }
    }

    /// Asks what to do with unsaved changes. Returns false if the user cancels.
    fn confirm_discard(&mut self) -> bool {
        if !self.editor.dirty {
            return true;
        }
        let answer = MessageDialog::new()
            .set_title("Unsaved changes")
            .set_description("Save changes to this document first?")
            .set_buttons(MessageButtons::YesNoCancel)
            .show();
        match answer {
            MessageDialogResult::Yes => self.save(false),
            MessageDialogResult::No => true,
            _ => false,
        }
    }

    /// Keyboard fallbacks for menu commands. The menu bar normally takes these
    /// keys first; this keeps them working if it doesn't.
    fn shortcuts(&mut self, ctx: &egui::Context) -> Vec<Command> {
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let keys = [
            (cmd_shift, Key::Z, Command::Redo),
            (cmd_shift, Key::S, Command::SaveAs),
            (cmd, Key::Z, Command::Undo),
            (cmd, Key::S, Command::Save),
            (cmd, Key::O, Command::Open),
            (cmd, Key::N, Command::New),
            (cmd, Key::A, Command::SelectAll),
            (cmd, Key::B, Command::Bold),
            (cmd, Key::I, Command::Italic),
            (cmd, Key::U, Command::Underline),
            (cmd, Key::T, Command::NextTheme),
            (cmd, Key::Equals, Command::ZoomIn),
            (cmd, Key::Plus, Command::ZoomIn),
            (cmd, Key::Minus, Command::ZoomOut),
            (cmd, Key::Period, Command::ToggleStatus),
            (cmd, Key::Slash, Command::Help),
        ];
        let mut out: Vec<Command> = keys
            .into_iter()
            .filter(|(m, k, _)| ctx.input_mut(|i| i.consume_key(*m, *k)))
            .map(|(.., c)| c)
            .collect();
        out.dedup();

        let digits = [
            Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4,
            Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9,
        ];
        for (n, key) in digits.into_iter().enumerate() {
            if ctx.input_mut(|i| i.consume_key(cmd, key))
                && let Some(style) = self.theme().style_for_key(n as u8).map(str::to_owned)
            {
                self.editor.set_style(&style);
            }
        }
        out
    }

    fn run(&mut self, command: Command, ctx: &egui::Context) {
        match command {
            Command::New => {
                if self.confirm_discard() {
                    self.editor = Editor::new(Document::default());
                    self.path = None;
                }
            }
            Command::Open => {
                if self.confirm_discard()
                    && let Some(p) = FileDialog::new().add_filter("Folio document", &["folio"]).pick_file()
                {
                    self.open_path(p);
                }
            }
            Command::Save => {
                self.save(false);
            }
            Command::SaveAs => {
                self.save(true);
            }
            // Goes through the normal close path, which asks about unsaved changes.
            Command::Quit => ctx.send_viewport_cmd(ViewportCommand::Close),
            Command::Undo => self.editor.undo(),
            Command::Redo => self.editor.redo(),
            Command::SelectAll => self.editor.select_all(),
            Command::Bold => self.editor.toggle(|f| f.bold, |f, on| f.bold = on),
            Command::Italic => self.editor.toggle(|f| f.italic, |f, on| f.italic = on),
            Command::Underline => self.editor.toggle(|f: &Fmt| f.underline, |f, on| f.underline = on),
            Command::NextTheme => {
                self.theme = (self.theme + 1) % self.themes.len();
                self.editor.dirty = true;
            }
            Command::ZoomIn => self.editor.zoom = (self.editor.zoom * 1.1).min(4.0),
            Command::ZoomOut => self.editor.zoom = (self.editor.zoom / 1.1).max(0.3),
            Command::ZoomReset => self.editor.zoom = 1.0,
            Command::ToggleStatus => self.show_status = !self.show_status,
            Command::Help => self.show_help = !self.show_help,
            Command::Component(i, j) => {
                if let Err(e) = self.components.run_command(i, j) {
                    self.error = Some(e);
                }
            }
        }
    }

    /// Talks to running components: answers their requests and tells them
    /// when the document changes.
    fn service_components(&mut self, now: f64) {
        let (calls, errors) = self.components.poll();
        if let Some(e) = errors.into_iter().last() {
            self.error = Some(e);
        }
        for call in calls {
            let component = call.component;
            let id = call.id.clone();
            let result = self.answer(call, now);
            match id {
                Some(id) => self.components.reply(component, id, result),
                None => {
                    if let Err(e) = result {
                        eprintln!("folio: component message failed: {e}");
                    }
                }
            }
        }

        // Typing changes the document constantly, so send at most ~3 updates a second.
        let (sent, at) = self.sent_revision;
        if self.editor.revision != sent && now - at > 0.3 {
            let doc = &self.editor.doc;
            self.components.emit(
                "changed",
                json!({ "words": doc.word_count(), "paragraphs": doc.paragraphs.len() }),
            );
            self.sent_revision = (self.editor.revision, now);
        }
    }

    fn answer(&mut self, call: Call, now: f64) -> Result<Value, String> {
        let needs = |permission: &str| {
            if self.components.has_permission(call.component, permission) {
                Ok(())
            } else {
                Err(format!("this needs the \"{permission}\" permission in component.toml"))
            }
        };
        let text_param = || {
            call.params
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| "missing \"text\"".to_owned())
        };

        match call.method.as_str() {
            "doc.get" => {
                needs(components::READ)?;
                let mut doc: Value = serde_json::from_str(&self.editor.doc.to_json()).map_err(|e| e.to_string())?;
                doc["path"] = json!(self.path);
                doc["theme"] = json!(self.theme().name);
                Ok(doc)
            }
            "doc.text" => {
                needs(components::READ)?;
                let doc = &self.editor.doc;
                Ok(json!({ "text": doc.text(Pos::default(), doc.end()) }))
            }
            "doc.insert" => {
                needs(components::EDIT)?;
                let text = text_param()?;
                self.editor.insert_at_cursor(&text, &self.themes[self.theme]);
                Ok(Value::Null)
            }
            "doc.replace" => {
                needs(components::EDIT)?;
                let mut params = call.params.clone();
                params["folio"] = json!(1);
                let mut doc = Document::from_json(&params.to_string())?;
                doc.theme = self.editor.doc.theme.clone();
                self.editor.replace_document(doc);
                Ok(Value::Null)
            }
            "notify" => {
                self.notice = Some((text_param()?, now));
                Ok(Value::Null)
            }
            other => Err(format!("unknown method \"{other}\"")),
        }
    }

    /// Draws the title strip in the theme's colors and lets it move the window.
    fn titlebar(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let colors = &self.themes[self.theme].colors;
        let bg = color(&colors.background);
        ui.painter().rect_filled(rect, 0.0, bg);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            &self.title,
            FontId::proportional(13.0),
            color(&colors.status),
        );

        let response = ui.interact(rect, egui::Id::unique("titlebar"), Sense::click_and_drag());
        if response.double_clicked() {
            let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        } else if response.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }

        // Match the traffic lights and system UI to a light or dark theme.
        if self.appearance_for != Some(bg) {
            let [r, g, b, _] = bg.to_array();
            let luma = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
            let system = if luma < 128.0 { SystemTheme::Dark } else { SystemTheme::Light };
            ui.ctx().send_viewport_cmd(ViewportCommand::SetTheme(system));
            self.appearance_for = Some(bg);
        }
    }

    fn overlays(&self, ui: &egui::Ui) {
        let rect = ui.max_rect();
        let painter = ui.painter();
        let faint = color(&self.theme().colors.status);

        if self.show_status {
            let words = self.editor.doc.word_count();
            let pages = self.editor.page_count;
            let mut text = format!(
                "{}   {} {}   {} {}   {:.0}%",
                self.theme().name,
                pages,
                if pages == 1 { "page" } else { "pages" },
                words,
                if words == 1 { "word" } else { "words" },
                self.editor.zoom * 100.0,
            );
            for item in self.components.status_items() {
                text = format!("{item}   {text}");
            }
            painter.text(
                rect.right_bottom() + egui::vec2(-14.0, -8.0),
                Align2::RIGHT_BOTTOM,
                text,
                FontId::proportional(11.0),
                faint,
            );
        }
        let now = ui.input(|i| i.time);
        if let Some((notice, at)) = &self.notice
            && self.error.is_none()
            && now - at < 5.0
        {
            painter.text(
                rect.left_bottom() + egui::vec2(14.0, -8.0),
                Align2::LEFT_BOTTOM,
                notice,
                FontId::proportional(12.0),
                faint,
            );
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
        }
        if let Some(err) = &self.error {
            painter.text(
                rect.left_bottom() + egui::vec2(14.0, -8.0),
                Align2::LEFT_BOTTOM,
                err,
                FontId::proportional(12.0),
                egui::Color32::from_rgb(230, 90, 80),
            );
        }
        if self.show_help {
            let galley = painter.layout_no_wrap(HELP.into(), FontId::monospace(12.0), egui::Color32::WHITE);
            let box_rect = egui::Rect::from_center_size(rect.center(), galley.size() + egui::vec2(36.0, 30.0));
            painter.rect_filled(box_rect, 8.0, egui::Color32::from_black_alpha(220));
            painter.galley(box_rect.min + egui::vec2(18.0, 15.0), galley, egui::Color32::WHITE);
        }
    }
}

impl eframe::App for Folio {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if ctx.input(|i| i.viewport().close_requested()) && !self.confirm_discard() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        }

        let mut commands = self.menu.poll();
        commands.extend(self.shortcuts(&ctx));
        for command in commands {
            self.run(command, &ctx);
        }
        self.service_components(ctx.input(|i| i.time));

        let full = ui.max_rect();
        let (bar, body) = full.split_top_bottom_at_y(full.top() + TITLEBAR);
        self.titlebar(ui, bar);
        let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body));
        self.editor.show(&mut body_ui, &self.themes[self.theme], &self.fonts);
        self.overlays(&body_ui);

        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("Untitled".to_owned(), |s| s.to_string_lossy().into_owned());
        let title = format!("{}{}", name, if self.editor.dirty { " •" } else { "" });
        // The native title is hidden but still used by Mission Control and the Window menu.
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

fn main() -> eframe::Result {
    let path = std::env::args_os().nth(1).map(PathBuf::from);
    let icon = eframe::icon_data::from_png_bytes(ICON_PNG).expect("bundled icon is a valid PNG");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            // eframe also uses the starting title as the app menu's name.
            .with_title("Folio")
            .with_icon(icon.clone())
            .with_inner_size([1000.0, 1100.0])
            // Draw under a transparent titlebar so the theme colors the whole window.
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false),
        ..Default::default()
    };
    eframe::run_native(
        "Folio",
        options,
        Box::new(move |cc| Ok(Box::new(Folio::new(cc, path, &icon)))),
    )
}
