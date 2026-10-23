//! Folio: a document editor that starts bare and gets out of the way.
//! How documents look comes from theme files; how Folio behaves comes from
//! settings.toml and keys.toml; everything extra comes from components.

mod components;
mod doc;
mod editor;
mod format;
mod keys;
mod menu;
mod panels;
mod settings;
mod theme;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Align2, FontId, Sense, SystemTheme, ViewportCommand};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult};
use serde_json::{Value, json};

use components::{Call, Host};
use doc::{Document, Fmt, Pos};
use editor::Editor;
use format::Format;
use keys::{Builtin, Command, Keymap};
use menu::{MenuBar, MenuContent};
use settings::{PanelKind, Settings};
use theme::{FontMap, Theme, color};

pub const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

/// Height of the strip drawn where the macOS titlebar would be.
const TITLEBAR: f32 = 28.0;

struct Folio {
    ctx: egui::Context,
    icon: egui::IconData,
    editor: Editor,
    path: Option<PathBuf>,
    themes: Vec<Theme>,
    theme: usize,
    fonts: FontMap,

    settings: Settings,
    user_keys: std::collections::BTreeMap<String, String>,
    keymap: Keymap,
    watch: settings::Watch,
    /// Problems found in config files, shown until they're fixed.
    config_problems: Vec<String>,
    status_format: Format,
    title_format: Format,
    /// Header and footer formats for each theme, by theme index.
    page_formats: Vec<(Format, Format)>,
    /// Parsed `lines` of each text panel, by panel index.
    panel_lines: Vec<Vec<Format>>,

    show_help: bool,
    title: String,
    error: Option<String>,
    menu: Option<MenuBar>,
    components: Host,
    /// A short message from a component, and when it arrived.
    notice: Option<(String, f64)>,
    /// Last document revision components were told about, and when.
    sent_revision: (u64, f64),
    /// Background color last used to pick a light or dark window appearance.
    appearance_for: Option<egui::Color32>,
}

impl Folio {
    fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>, icon: egui::IconData) -> Self {
        settings::write_examples();
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);

        let ctx = cc.egui_ctx.clone();
        let waker = ctx.clone();
        let host = Host::new(
            components::discover(),
            Arc::new(move || waker.request_repaint()),
            Box::new(components::approve_with_dialog),
        );

        let mut app = Self {
            ctx,
            icon,
            editor: Editor::new(Document::default()),
            path: None,
            themes: Vec::new(),
            theme: 0,
            fonts: FontMap::default(),
            settings: Settings::default(),
            user_keys: Default::default(),
            keymap: Keymap::default(),
            watch: settings::Watch::new(),
            config_problems: Vec::new(),
            status_format: Format::default(),
            title_format: Format::default(),
            page_formats: Vec::new(),
            panel_lines: Vec::new(),
            show_help: false,
            title: String::new(),
            error: None,
            menu: None,
            components: host,
            notice: None,
            sent_revision: (0, 0.0),
            appearance_for: None,
        };
        app.load_config(true);
        if let Some(p) = path {
            app.open_path(p);
        }
        app
    }

    fn theme(&self) -> &Theme {
        &self.themes[self.theme]
    }

    // ---- Config ---------------------------------------------------------------

    /// Loads (or reloads) themes, settings.toml and keys.toml. The files are
    /// the source of truth, so a reload applies everything in them.
    fn load_config(&mut self, first: bool) {
        let current = self.themes.get(self.theme).map(|t| t.name.clone());
        self.themes = theme::load_themes();
        self.fonts = theme::install_fonts(&self.ctx, &self.themes);

        let (settings, mut problems) = settings::load();
        let (user_keys, key_problems) = settings::load_keys();
        problems.extend(key_problems);

        // On reload, keep the current theme unless settings.toml names a different one.
        let previous_setting = self.settings.theme.clone();
        let wanted = if first || settings.theme != previous_setting {
            settings.theme.clone().or(current)
        } else {
            current
        };
        self.theme = wanted
            .and_then(|name| self.themes.iter().position(|t| t.name == name))
            .unwrap_or(0);
        self.editor.zoom = settings.zoom.clamp(0.3, 4.0);

        self.status_format = Format::parse_or_message(&settings.status);
        self.title_format = Format::parse_or_message(&settings.title);
        self.page_formats = self
            .themes
            .iter()
            .map(|t| {
                let parse = |s: &Option<String>| s.as_deref().map(Format::parse_or_message).unwrap_or_default();
                (parse(&t.page.header), parse(&t.page.footer))
            })
            .collect();
        self.panel_lines = settings
            .panels
            .iter()
            .map(|p| p.lines.iter().map(|l| Format::parse_or_message(l)).collect())
            .collect();

        self.settings = settings;
        self.user_keys = user_keys;
        self.config_problems = problems;
        self.rebuild_keys();
    }

    /// Rebuilds the keymap and menu bar; needed when keys or the theme change,
    /// since each theme brings its own paragraph styles and shortcuts.
    fn rebuild_keys(&mut self) {
        let theme = &self.themes[self.theme];
        let mut defaults: Vec<(String, Command)> = theme
            .style_keys()
            .map(|(keys, style)| (keys, Command::Style(style)))
            .collect();
        for c in &self.components.components {
            for cmd in &c.manifest.commands {
                if let Some(keys) = &cmd.keys {
                    defaults.push((keys.clone(), Command::Component(c.manifest.name.clone(), cmd.id.clone())));
                }
            }
        }
        let (keymap, problems) = Keymap::build(&self.user_keys, defaults);
        self.keymap = keymap;
        self.config_problems.retain(|p| !p.starts_with("keys.toml"));
        self.config_problems.extend(problems);

        let mut styles: Vec<(&String, &theme::Style)> = theme.styles.iter().collect();
        styles.sort_by_key(|(name, s)| (s.key.unwrap_or(u8::MAX), name.as_str()));
        let content = MenuContent {
            styles: styles.into_iter().map(|(n, _)| n.clone()).collect(),
            components: self
                .components
                .components
                .iter()
                .map(|c| {
                    let cmds = c.manifest.commands.iter().map(|cmd| (cmd.id.clone(), cmd.label.clone())).collect();
                    (c.manifest.name.clone(), cmds)
                })
                .collect(),
            keymap: &self.keymap,
        };
        self.menu = Some(MenuBar::install(&self.ctx, &self.icon, &content));
    }

    /// Writes a setting back to settings.toml so Folio remembers it.
    fn persist(&mut self, write: impl FnOnce() -> Result<(), String>) {
        if let Err(e) = write() {
            self.error = Some(e);
        }
        self.watch.forget_own_writes();
    }

    fn set_theme(&mut self, i: usize, remember: bool) {
        if i == self.theme {
            return;
        }
        self.theme = i;
        self.rebuild_keys();
        if remember {
            let name = self.theme().name.clone();
            self.settings.theme = Some(name.clone());
            self.persist(|| settings::save_value("theme", name));
        }
    }

    // ---- Files ----------------------------------------------------------------

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
                    self.set_theme(i, false);
                }
                let zoom = self.editor.zoom;
                self.editor = Editor::new(doc);
                self.editor.zoom = zoom;
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

    // ---- Commands -------------------------------------------------------------

    fn run(&mut self, command: Command, now: f64) {
        match command {
            Command::Builtin(b) => self.run_builtin(b, now),
            Command::Style(style) => {
                if self.theme().styles.contains_key(&style) {
                    self.editor.set_style(&style);
                } else {
                    self.error = Some(format!("The {} theme has no \"{style}\" style", self.theme().name));
                }
            }
            Command::Component(name, id) => {
                if let Err(e) = self.components.run_command(&name, &id) {
                    self.error = Some(e);
                }
            }
        }
    }

    fn run_builtin(&mut self, b: Builtin, now: f64) {
        match b {
            Builtin::New => {
                if self.confirm_discard() {
                    let zoom = self.editor.zoom;
                    self.editor = Editor::new(Document::default());
                    self.editor.zoom = zoom;
                    self.path = None;
                }
            }
            Builtin::Open => {
                if self.confirm_discard()
                    && let Some(p) = FileDialog::new().add_filter("Folio document", &["folio"]).pick_file()
                {
                    self.open_path(p);
                }
            }
            Builtin::Save => {
                self.save(false);
            }
            Builtin::SaveAs => {
                self.save(true);
            }
            // Goes through the normal close path, which asks about unsaved changes.
            Builtin::Quit => self.ctx.send_viewport_cmd(ViewportCommand::Close),
            Builtin::Undo => self.editor.undo(),
            Builtin::Redo => self.editor.redo(),
            Builtin::SelectAll => self.editor.select_all(),
            Builtin::Bold => self.editor.toggle(|f| f.bold, |f, on| f.bold = on),
            Builtin::Italic => self.editor.toggle(|f| f.italic, |f, on| f.italic = on),
            Builtin::Underline => self.editor.toggle(|f: &Fmt| f.underline, |f, on| f.underline = on),
            Builtin::NextTheme => {
                let next = (self.theme + 1) % self.themes.len();
                self.set_theme(next, true);
                self.editor.dirty = true;
            }
            Builtin::ZoomIn | Builtin::ZoomOut | Builtin::ZoomReset => {
                let z = self.editor.zoom;
                self.editor.zoom = match b {
                    Builtin::ZoomIn => (z * 1.1).min(4.0),
                    Builtin::ZoomOut => (z / 1.1).max(0.3),
                    _ => 1.0,
                };
                let zoom = (f64::from(self.editor.zoom) * 100.0).round() / 100.0;
                self.persist(|| settings::save_value("zoom", zoom));
            }
            Builtin::ToggleStatus => {
                self.settings.show_status = !self.settings.show_status;
                let on = self.settings.show_status;
                self.persist(|| settings::save_value("show_status", on));
            }
            Builtin::TogglePanels => {
                if self.settings.panels.is_empty() {
                    self.notice = Some(("No panels yet. Add [[panels]] to settings.toml (Folio → Open Config Folder)".into(), now));
                    return;
                }
                self.settings.show_panels = !self.settings.show_panels;
                let on = self.settings.show_panels;
                self.persist(|| settings::save_value("show_panels", on));
            }
            Builtin::Help => self.show_help = !self.show_help,
            Builtin::ReloadConfig => {
                self.load_config(false);
                self.watch.forget_own_writes();
                self.notice = Some(("Config reloaded".into(), now));
            }
            Builtin::OpenConfigFolder => {
                settings::write_examples();
                if let Err(e) = std::process::Command::new("open").arg(settings::config_dir()).spawn() {
                    self.error = Some(format!("Couldn't open the config folder: {e}"));
                }
            }
        }
    }

    fn panel_action(&mut self, action: panels::Action, now: f64) {
        use panels::Action;
        match action {
            Action::Goto(para) => return self.editor.goto(para),
            Action::Run(command) => return self.run(command, now),
            Action::Move(i, side) => self.settings.panels[i].side = side,
            Action::Close(i) => {
                self.settings.panels.remove(i);
                self.panel_lines.remove(i);
            }
            Action::Resize(i, size) => self.settings.panels[i].size = size.round(),
        }
        // Layout changes go back into settings.toml.
        let panels = self.settings.panels.clone();
        self.persist(|| settings::save_panels(&panels));
    }

    // ---- Fields for the formatting language ------------------------------------

    /// Every field's current value. Built once per frame; cheap next to layout.
    fn fields(&self) -> HashMap<String, String> {
        let doc = &self.editor.doc;
        let words = doc.word_count();
        let chars: usize = doc.paragraphs.iter().map(|p| p.len()).sum();
        let (date, time) = local_date_time();
        let mut f: HashMap<String, String> = HashMap::from([
            ("words".into(), words.to_string()),
            ("chars".into(), chars.to_string()),
            ("paragraphs".into(), doc.paragraphs.len().to_string()),
            ("page".into(), self.editor.caret_page.to_string()),
            ("pages".into(), self.editor.page_count.to_string()),
            ("theme".into(), self.theme().name.clone()),
            ("zoom".into(), format!("{:.0}", self.editor.zoom * 100.0)),
            ("style".into(), doc.paragraphs[self.editor.cursor().para].style.clone()),
            ("reading_time".into(), format!("{} min", words.div_ceil(230).max(1))),
            ("date".into(), date),
            ("time".into(), time),
        ]);
        let file = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("Untitled".to_owned(), |s| s.to_string_lossy().into_owned());
        f.insert("file".into(), file);
        if let Some(p) = &self.path {
            f.insert("path".into(), p.display().to_string());
        }
        if self.editor.dirty {
            f.insert("dirty".into(), "1".into());
        }
        if let Some(sel) = self.editor.selected_text() {
            f.insert("selection_words".into(), sel.split_whitespace().count().to_string());
        }
        let statuses: Vec<&str> = self.components.status_items().collect();
        if !statuses.is_empty() {
            f.insert("component_status".into(), statuses.join("   "));
        }
        for c in &self.components.components {
            if let Some(s) = &c.status {
                f.insert(format!("component:{}", c.manifest.name.to_lowercase()), s.clone());
            }
        }
        f
    }

    // ---- Components -----------------------------------------------------------

    /// Talks to running components: answers their requests and tells them
    /// when the document changes.
    fn service_components(&mut self, now: f64) {
        // Component panels on screen need their component running.
        if self.settings.show_panels {
            let wanted: Vec<usize> = self
                .settings
                .panels
                .iter()
                .filter(|p| p.kind == PanelKind::Component)
                .filter_map(|p| self.components.index_of(p.component.as_deref()?))
                .collect();
            for i in wanted {
                if let Err(e) = self.components.ensure_running(i) {
                    self.error = Some(e);
                }
            }
        }

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

        // Typing changes the document constantly, so send at most ~3 updates a
        // second. A component that just started gets the current state right away.
        let (sent, at) = self.sent_revision;
        let fresh = std::mem::take(&mut self.components.just_started);
        if fresh || (self.editor.revision != sent && now - at > 0.3) {
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

    // ---- Drawing --------------------------------------------------------------

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

    fn overlays(&self, ui: &egui::Ui, fields: &dyn Fn(&str) -> Option<String>) {
        let rect = ui.max_rect();
        let painter = ui.painter();
        let faint = color(&self.theme().colors.status);

        if self.settings.show_status {
            painter.text(
                rect.right_bottom() + egui::vec2(-14.0, -8.0),
                Align2::RIGHT_BOTTOM,
                self.status_format.eval(fields),
                FontId::proportional(11.0),
                faint,
            );
        }

        let now = ui.input(|i| i.time);
        let problem = self.error.clone().or_else(|| self.config_problems.first().cloned());
        if let Some(err) = problem {
            painter.text(
                rect.left_bottom() + egui::vec2(14.0, -8.0),
                Align2::LEFT_BOTTOM,
                err,
                FontId::proportional(12.0),
                egui::Color32::from_rgb(230, 90, 80),
            );
        } else if let Some((notice, at)) = &self.notice
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

        if self.show_help {
            let rows = self.keymap.describe();
            let width = rows.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
            let mut text: String = rows.iter().map(|(k, c)| format!("{k:width$}   {c}\n")).collect();
            text += "\nRemap any of these in keys.toml (Folio → Open Config Folder)";
            let galley = painter.layout_no_wrap(text, FontId::monospace(12.0), egui::Color32::WHITE);
            let box_rect = egui::Rect::from_center_size(rect.center(), galley.size() + egui::vec2(36.0, 30.0));
            painter.rect_filled(box_rect, 8.0, egui::Color32::from_black_alpha(225));
            painter.galley(box_rect.min + egui::vec2(18.0, 15.0), galley, egui::Color32::WHITE);
        }
    }
}

impl eframe::App for Folio {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);

        if ctx.input(|i| i.viewport().close_requested()) && !self.confirm_discard() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        }
        if self.watch.changed(now) {
            self.load_config(false);
        }
        // Check the config files again even when idle, so edits show up.
        ctx.request_repaint_after(std::time::Duration::from_secs(1));

        let mut commands = self.menu.as_ref().map(MenuBar::poll).unwrap_or_default();
        commands.extend(self.keymap.take_commands(&ctx));
        for command in commands {
            self.run(command, now);
        }
        self.service_components(now);

        let full = ui.max_rect();
        let (bar, body) = full.split_top_bottom_at_y(full.top() + TITLEBAR);
        self.titlebar(ui, bar);
        let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body));

        let values = self.fields();
        let field = |name: &str| values.get(name).filter(|v| !v.is_empty()).cloned();

        if self.settings.show_panels && !self.settings.panels.is_empty() {
            let cx = panels::Context {
                doc: &self.editor.doc,
                theme: &self.themes[self.theme],
                cursor_para: self.editor.cursor().para,
                text_lines: &self.panel_lines,
                fields: &field,
                host: &self.components,
            };
            let actions = panels::show(&mut body_ui, &self.settings.panels, &cx);
            for action in actions {
                self.panel_action(action, now);
            }
        }

        let rest = body_ui.available_rect_before_wrap();
        let mut page_ui = body_ui.new_child(egui::UiBuilder::new().max_rect(rest));
        let (header, footer) = &self.page_formats[self.theme];
        let page_text = |page: usize, pages: usize| {
            let page_field = |name: &str| match name {
                "page" => Some(page.to_string()),
                "pages" => Some(pages.to_string()),
                other => field(other),
            };
            (header.eval(&page_field), footer.eval(&page_field))
        };
        self.editor.show(&mut page_ui, &self.themes[self.theme], &self.fonts, &page_text);

        let values = self.fields();
        let field = |name: &str| values.get(name).filter(|v| !v.is_empty()).cloned();
        self.overlays(&page_ui, &field);

        let title = self.title_format.eval(&field);
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

/// Local date (YYYY-MM-DD) and time (HH:MM).
fn local_date_time() -> (String, String) {
    // SAFETY: `time` and `localtime_r` only write to the values we pass in.
    let tm = unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        tm
    };
    (
        format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday),
        format!("{:02}:{:02}", tm.tm_hour, tm.tm_min),
    )
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
        Box::new(move |cc| Ok(Box::new(Folio::new(cc, path, icon)))),
    )
}
