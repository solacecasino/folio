//! text9000: a document editor that starts bare and gets out of the way.
//! How documents look comes from theme files; how text9000 behaves comes from
//! settings.toml and keys.toml; everything extra comes from adapters.

// Release builds on Windows are window apps, with no console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod adapter_window;
mod adapters;
#[cfg(unix)]
mod alloc;
mod clipboard;
mod dialog;
mod doc;
mod editor;
mod find;
mod format;
mod keys;
mod links;
mod log;
mod menu;
mod panels;
mod pdf;
mod platform;
mod plugins;
mod safety;
mod scripts;
mod settings;
mod series;
mod shell;
mod spell;
mod state;
mod theme;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use egui::{self, Align2, FontId, Sense, SystemTheme, ViewportCommand};
use rfd::{FileDialog, MessageButtons, MessageDialogResult};
use serde_json::{Value, json};

use adapters::{Call, Host};
use doc::{Document, Fmt, Pos};
use editor::Editor;
use format::Format;
use keys::{Builtin, Command, Keymap};
use menu::{MenuBar, MenuContent};
use settings::{PanelKind, Settings};
use theme::{FontMap, Theme, color};

// 256px: the window icon, dock icon and About box never show it larger, and
// the 1024px original cost over 30 MB once macOS scaled it for the dock.
pub const ICON_PNG: &[u8] = include_bytes!("../assets/icon-256.png");

/// Height of the strip drawn where the macOS titlebar would be (the menus
/// on Windows and Linux).
const TITLEBAR: f32 = 28.0;

struct Text9000 {
    ctx: egui::Context,
    icon: egui::IconData,
    editor: Editor,
    path: Option<PathBuf>,
    themes: Vec<Theme>,
    theme: usize,
    fonts: FontMap,
    /// Fonts for a theme just switched to: egui loads them for the next
    /// pass, so they replace `fonts` there (see `load_fonts`).
    next_fonts: Option<FontMap>,
    /// The largest texture egui may make (see `raw_input_hook`), and the zoom
    /// and screen scale its text texture was last rebuilt for.
    texture_side: usize,
    texture_scale: Option<(f32, f32)>,
    scale_changed_at: Option<f64>,

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
    find: find::FindBar,
    /// Whether the find bar had the keyboard last frame.
    find_focused: bool,
    show_versions: bool,
    /// Help → Log, when open.
    log_window: Option<log::Window>,
    /// The last error written to the log, so each is logged once.
    logged_error: Option<String>,
    show_adapters: bool,
    /// The macOS spell checker, when spell checking is on.
    speller: Option<spell::Speller>,
    /// This document's saved versions, newest first, and whether to reload them.
    versions: Vec<safety::Version>,
    versions_stale: bool,
    /// Revision last seen, when it changed, and the last revision the
    /// safety net handled (autosave or recovery).
    seen_revision: (u64, f64),
    safety_revision: u64,
    last_auto_version: f64,
    /// `doc.print_model` requests, answered during the frame where text
    /// layout is available: (adapter, request id).
    pending_prints: Vec<(usize, Value)>,
    /// A question an adapter asked with `ui.prompt`, waiting for an answer.
    prompt: Option<Prompt>,
    /// (revision, cursor, anchor) last sent in a `snapshot` event.
    sent_snapshot: Option<(u64, Pos, Pos)>,
    title: String,
    error: Option<String>,
    menu: Option<MenuBar>,
    adapters: Host,
    /// Window placement and recent documents, saved in state.json.
    state: state::State,
    /// The window's latest placement and when it last changed, until saved.
    window_moved: Option<(state::WindowRect, f64)>,
    /// The first-launch check that the window fits the screen has run.
    window_fitted: bool,
    /// WebAssembly plugins (nothing in folio-lite).
    plugins: plugins::Plugins,
    scripts: scripts::Scripts,
    /// The last plugin edit applied: (revision it was made against, the
    /// revision after applying it), so a plugin's several inserts from one
    /// event all apply.
    plugin_edit: Option<(u64, u64)>,
    /// A short message from an adapter, and when it arrived.
    notice: Option<(String, f64)>,
    /// Last document revision adapters were told about, and when.
    sent_revision: (u64, f64),
    /// Background color last used to pick a light or dark window appearance.
    appearance_for: Option<egui::Color32>,
}

impl Text9000 {
    fn new(egui_ctx: &egui::Context, path: Option<PathBuf>, icon: egui::IconData, state: state::State) -> Self {
        settings::write_examples();
        links::install(egui_ctx);
        egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);

        let ctx = egui_ctx.clone();
        let waker = ctx.clone();
        let host = Host::new(
            adapters::discover(),
            Arc::new(move || waker.request_repaint()),
            Box::new(adapters::approve_with_dialog),
        );

        let mut app = Self {
            ctx,
            icon,
            editor: Editor::new(Document::default()),
            path: None,
            themes: Vec::new(),
            theme: 0,
            fonts: FontMap::default(),
            next_fonts: None,
            texture_side: 2048,
            texture_scale: None,
            scale_changed_at: None,
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
            find: find::FindBar::default(),
            find_focused: false,
            show_versions: false,
            log_window: None,
            logged_error: None,
            show_adapters: false,
            speller: None,
            versions: Vec::new(),
            versions_stale: true,
            seen_revision: (0, 0.0),
            safety_revision: 0,
            last_auto_version: f64::NEG_INFINITY,
            pending_prints: Vec::new(),
            prompt: None,
            sent_snapshot: None,
            title: String::new(),
            error: None,
            menu: None,
            adapters: host,
            plugins: plugins::Plugins::start(egui_ctx),
            scripts: scripts::Scripts::default(),
            plugin_edit: None,
            state,
            window_moved: None,
            window_fitted: false,
            notice: None,
            sent_revision: (0, 0.0),
            appearance_for: None,
        };
        app.load_config(true);
        if let Some(from) = platform::ADOPTED.get() {
            app.notice = Some((format!("Welcome to text9000. Your Folio settings came along from {}", platform::hide_home(&from.display().to_string())), 0.0));
        }
        if let Some(p) = path {
            app.open_file(p, 0.0);
        }
        if let Some(crash) = log::last_crash() {
            log::push("text9000", log::Level::Error, format!("text9000 ran into a bug last time:\n{crash}"));
            app.error = Some("text9000 ran into a bug last time. Help → Report a Problem sends us the details.".into());
        }
        if let Some(problem) = platform::dialog_problem() {
            app.error = Some(problem);
        }
        app.offer_recovery();
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
        self.load_fonts(first);
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

        let language_changed = settings.spell_language != self.settings.spell_language;
        if !settings.spellcheck {
            self.speller = None;
        } else if self.speller.is_none() || language_changed || first {
            self.speller = Some(spell::Speller::new(&settings.spell_language));
        }
        self.settings = settings;
        self.user_keys = user_keys;
        self.config_problems = problems;
        // init.lua runs at startup, on Reload Config, and when it's saved,
        // before the menus are built so they list its commands.
        let loaded = self.scripts.reload_if_changed(&mut self.editor, &self.themes[self.theme], self.path.as_deref());
        self.lua_result(loaded);
        self.rebuild_keys();
    }

    /// Loads only the current theme's fonts: each theme's fonts take a few
    /// MB, and only one theme shows at a time. Outside the first frame, egui
    /// only has them from the next pass, so until then the old map stays
    /// (unknown faces fall back to the built-in font rather than panic) and
    /// the pass is redone at once, so the switch never shows the wrong font.
    fn load_fonts(&mut self, first: bool) {
        let fonts = theme::install_fonts(&self.ctx, std::slice::from_ref(&self.themes[self.theme]));
        if first {
            self.fonts = fonts;
        } else {
            self.next_fonts = Some(fonts);
            self.ctx.request_discard("theme fonts");
        }
    }

    /// After zooming (or moving to a screen with another scale), the text
    /// texture still holds letters at every size passed through, up to its
    /// 16 MB cap. egui starts a fresh one only when its text options change,
    /// so once the zoom has settled for a second, the texture size limit
    /// switches between 2048 and 2047 (no visible difference).
    fn trim_text_texture(&mut self, ctx: &egui::Context, now: f64) {
        let scale = Some((self.editor.zoom, ctx.pixels_per_point()));
        if scale != self.texture_scale {
            // The first frame only notes the scale: there's nothing to trim yet.
            if self.texture_scale.is_some() {
                self.scale_changed_at = Some(now);
            }
            self.texture_scale = scale;
        }
        if let Some(t) = self.scale_changed_at {
            if now - t >= 1.0 {
                self.scale_changed_at = None;
                self.texture_side = if self.texture_side == 2048 { 2047 } else { 2048 };
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(1.0 - (now - t)));
            }
        }
    }

    /// Rebuilds the keymap and menu bar; needed when keys or the theme change,
    /// since each theme brings its own paragraph styles and shortcuts.
    fn rebuild_keys(&mut self) {
        let theme = &self.themes[self.theme];
        let mut defaults: Vec<(String, Command)> = theme
            .style_keys()
            .map(|(keys, style)| (keys, Command::Style(style)))
            .collect();
        for (_, c) in self.adapters.installed() {
            for cmd in &c.manifest.commands {
                if let Some(keys) = &cmd.keys {
                    defaults.push((keys.clone(), Command::Adapter(c.manifest.name.clone(), cmd.id.clone())));
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
            adapters: self
                .adapters
                .installed()
                .map(|(_, c)| {
                    let cmds = c
                        .manifest
                        .commands
                        .iter()
                        .map(|cmd| menu::AdapterItem {
                            id: cmd.id.clone(),
                            label: cmd.label.clone(),
                            menu: cmd.menu.clone(),
                        })
                        .collect();
                    (c.manifest.name.clone(), cmds)
                })
                .collect(),
            plugins: self.plugins.commands(),
            scripts: self.scripts.menu_items(),
            apps: series::SIBLINGS
                .iter()
                .filter(|s| series::find(s, self.settings.apps.get(s.name).map(String::as_str)).is_some())
                .map(|s| s.name.to_owned())
                .collect(),
            recent: self.state.recent.clone(),
            keymap: &self.keymap,
        };
        self.menu = Some(MenuBar::install(&self.ctx, &self.icon, &content));
    }

    /// Writes a setting back to settings.toml so text9000 remembers it.
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
        self.load_fonts(false);
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
                self.versions_stale = true;
                self.adapters.emit("opened", json!({ "path": path }));
                self.remember(&path);
                log::push("text9000", log::Level::Info, format!("Opened {}", path.display()));
                self.path = Some(path);
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Couldn't open {}: {e}", path.display())),
        }
    }

    /// Adds a document to Open Recent.
    fn remember(&mut self, path: &std::path::Path) {
        self.state.add_recent(path);
        self.state.save();
        self.rebuild_keys();
    }

    /// Saves the window's size and place once it has stopped changing. On
    /// a first launch it also makes sure the window fits the screen, since
    /// the default size is taller than small laptop screens.
    fn track_window(&mut self, ctx: &egui::Context, now: f64) {
        if !self.window_fitted {
            let fit = ctx.input(|i| {
                let v = i.viewport();
                Some((v.monitor_size?, v.outer_rect?, v.inner_rect?))
            });
            if let Some((monitor, outer, inner)) = fit {
                self.window_fitted = true;
                // Room for the title bar, dock or taskbar.
                let chrome = outer.height() - inner.height();
                let max_h = monitor.y * 0.9 - chrome;
                if self.state.window.is_none() && inner.height() > max_h && max_h > 300.0 {
                    let size = egui::vec2(inner.width().min(monitor.x * 0.9), max_h);
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
                    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(egui::pos2(
                        ((monitor.x - size.x) / 2.0).max(0.0),
                        (monitor.y * 0.04).max(0.0),
                    )));
                }
            }
        }
        let rect = ctx.input(|i| {
            let v = i.viewport();
            let (outer, inner) = (v.outer_rect?, v.inner_rect?);
            (v.fullscreen != Some(true) && v.maximized != Some(true)).then_some(state::WindowRect {
                x: outer.min.x,
                y: outer.min.y,
                width: inner.width(),
                height: inner.height(),
            })
        });
        if let Some(rect) = rect
            && self.state.window != Some(rect)
            && self.window_moved.is_none_or(|(r, _)| r != rect)
        {
            self.window_moved = Some((rect, now));
        }
        if let Some((rect, at)) = self.window_moved
            && now - at > 1.0
        {
            self.state.window = Some(rect);
            self.state.save();
            self.window_moved = None;
        }
        if self.window_moved.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(1100));
        }
    }

    fn save(&mut self, choose: bool) -> bool {
        let path = match (&self.path, choose) {
            (Some(p), false) => p.clone(),
            _ => match FileDialog::new()
                .add_filter("text9000 document", &[series::EXTENSION])
                .add_filter("Folio document", &[series::FOLIO_EXTENSION])
                .set_file_name(format!("Untitled.{}", series::EXTENSION))
                .save_file()
            {
                Some(p) => p,
                None => return false,
            },
        };
        self.write_doc(path, true)
    }

    /// Writes the document to `path`, with a version snapshot if asked.
    fn write_doc(&mut self, path: PathBuf, version: bool) -> bool {
        self.editor.doc.theme = Some(self.theme().name.clone());
        if self.editor.doc.id.is_none() {
            self.editor.doc.id = Some(safety::new_doc_id());
        }
        match std::fs::write(&path, self.editor.doc.to_json()) {
            Ok(()) => {
                self.adapters.emit("saved", json!({ "path": path }));
                self.plugins.saved(&path);
                if self.path.as_ref() != Some(&path) {
                    self.remember(&path);
                }
                self.path = Some(path);
                self.editor.dirty = false;
                self.error = None;
                safety::clear_recovery();
                if version && self.settings.versions > 0 {
                    if let Err(e) = safety::save_version(&self.editor.doc, self.settings.versions) {
                        self.error = Some(format!("Couldn't keep a version: {e}"));
                    }
                    self.versions_stale = true;
                }
                true
            }
            Err(e) => {
                self.error = Some(format!("Couldn't save: {e}"));
                false
            }
        }
    }

    /// Autosaves named documents, or keeps a recovery copy of unsaved work,
    /// once typing pauses for two seconds.
    fn safety_net(&mut self, now: f64) {
        let rev = self.editor.revision;
        if rev != self.seen_revision.0 {
            self.seen_revision = (rev, now);
        }
        let quiet = now - self.seen_revision.1 > 2.0;
        if !self.editor.dirty || !quiet || self.safety_revision == rev {
            return;
        }
        self.safety_revision = rev;
        match self.path.clone() {
            Some(path) if self.settings.autosave => {
                // Autosaves add a version at most every five minutes.
                let version = now - self.last_auto_version > 300.0;
                if self.write_doc(path, version) && version {
                    self.last_auto_version = now;
                }
            }
            path => {
                if let Err(e) = safety::write_recovery(&self.editor.doc, path.as_deref()) {
                    self.error = Some(format!("Couldn't keep a recovery copy: {e}"));
                }
            }
        }
    }

    /// Offers back work left behind when text9000 didn't quit cleanly.
    fn offer_recovery(&mut self) {
        let Some((file, record)) = safety::orphaned_recoveries().into_iter().next() else { return };
        let restore = dialog::ask("Restore unsaved work?", &record.describe(), MessageButtons::YesNo)
            == MessageDialogResult::Yes;
        if restore {
            match record.document() {
                Ok(doc) => {
                    let zoom = self.editor.zoom;
                    self.editor = Editor::new(doc);
                    self.editor.zoom = zoom;
                    self.editor.dirty = true;
                    self.path = record.path.clone();
                    // Keep a copy under this window's name until it's saved.
                    let _ = safety::write_recovery(&self.editor.doc, self.path.as_deref());
                }
                Err(e) => {
                    self.error = Some(format!("Couldn't restore: {e}"));
                    return;
                }
            }
        }
        let _ = std::fs::remove_file(file);
    }

    fn restore_version(&mut self, file: &std::path::Path) {
        let Ok(mut doc) = std::fs::read_to_string(file).map_err(|e| e.to_string()).and_then(|s| Document::from_json(&s))
        else {
            self.error = Some("Couldn't read that version".into());
            return;
        };
        let confirm = dialog::ask(
            "Restore this version?",
            "The document will be replaced with this version. You can undo it, and your current text \
             is kept as a version the next time you save.",
            MessageButtons::OkCancel,
        );
        if confirm == MessageDialogResult::Ok {
            doc.id = self.editor.doc.id.clone();
            doc.theme = self.editor.doc.theme.clone();
            self.editor.replace_document(doc);
        }
    }

    fn refresh_versions(&mut self) {
        let wanted = self.show_versions
            || (self.settings.show_panels && self.settings.panels.iter().any(|p| p.kind == PanelKind::Versions));
        if wanted && self.versions_stale {
            self.versions = self.editor.doc.id.as_deref().map(safety::list_versions).unwrap_or_default();
            self.versions_stale = false;
        }
    }

    /// Asks what to do with unsaved changes. Returns false if the user cancels.
    fn confirm_discard(&mut self) -> bool {
        if !self.editor.dirty {
            return true;
        }
        let answer = dialog::ask("Unsaved changes", "Save changes to this document first?", MessageButtons::YesNoCancel);
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
            Command::Adapter(name, id) => {
                if let Err(e) = self.adapters.run_command(&name, &id) {
                    self.error = Some(e);
                }
            }
            Command::Plugin(name, id) => {
                if let Err(e) = self.plugins.run_command(&name, &id) {
                    self.error = Some(e);
                }
            }
            Command::Lua(name) => {
                let ran = self.scripts.run(&name, &mut self.editor, &self.themes[self.theme], self.path.as_deref());
                self.lua_result(ran);
                self.editor.reset_caret(now);
            }
            Command::App(name) => {
                if let Some(sibling) = series::sibling(&name) {
                    self.hand_off(sibling, None, now);
                }
            }
            Command::OpenFile(path) => {
                if self.confirm_discard() {
                    self.open_path(path);
                }
            }
        }
    }

    /// Shows a Lua error in the status line. The full error is already in
    /// the Log (scripts.rs logs it), so it isn't logged a second time.
    /// Also shows a script's `folio.notify` message.
    fn lua_result(&mut self, result: Result<(), String>) {
        if let Some(text) = self.scripts.take_notice() {
            self.notice = Some((text, self.ctx.input(|i| i.time)));
        }
        if let Err(e) = result {
            self.error = Some(format!("Lua: {} (Help → Log has the details)", e.lines().next().unwrap_or_default()));
            self.logged_error = self.error.clone();
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
                    self.versions_stale = true;
                    safety::clear_recovery();
                }
            }
            Builtin::Open => {
                let media: Vec<&str> = series::SIBLINGS.iter().flat_map(|s| s.extensions.iter().copied()).collect();
                if let Some(p) = FileDialog::new()
                    .add_filter("text9000 or Folio document", &[series::EXTENSION, series::FOLIO_EXTENSION])
                    .add_filter("Video, sound or photo (opens in its 9000 app)", &media)
                    .pick_file()
                {
                    self.open_file(p, now);
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
            // During a shared session the sharing adapter undoes only your own edits.
            Builtin::Undo | Builtin::Redo => match self.adapters.session {
                Some((adapter, true)) => {
                    self.adapters.notify(adapter, if b == Builtin::Undo { "undo" } else { "redo" });
                }
                _ if b == Builtin::Undo => self.editor.undo(),
                _ => self.editor.redo(),
            },
            Builtin::SelectAll => self.editor.select_all(),
            Builtin::Find | Builtin::Replace => {
                self.find.open(b == Builtin::Replace, self.editor.selected_text());
            }
            Builtin::FindNext | Builtin::FindPrevious => {
                if !self.find.open || self.find.query.is_empty() {
                    self.find.open(false, self.editor.selected_text());
                }
                self.find_step(b == Builtin::FindNext);
            }
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
                    self.notice = Some(("No panels yet. Add [[panels]] to settings.toml (text9000 → Open Config Folder)".into(), now));
                    return;
                }
                self.settings.show_panels = !self.settings.show_panels;
                let on = self.settings.show_panels;
                self.persist(|| settings::save_value("show_panels", on));
            }
            Builtin::Help => self.show_help = !self.show_help,
            // The guide is the README; the browser loads it, not text9000.
            Builtin::Guide => {
                if let Err(e) = platform::open(GUIDE_URL.as_ref()) {
                    self.error = Some(format!("Couldn't open the guide ({GUIDE_URL}): {e}"));
                }
            }
            Builtin::ReportProblem => {
                self.ctx.copy_text(self.debug_info());
                match platform::open(REPORT_URL.as_ref()) {
                    Ok(()) => self.notice = Some(("Debug info copied. Paste it into the form".into(), now)),
                    Err(e) => self.error = Some(format!("Couldn't open {REPORT_URL}: {e}")),
                }
            }
            Builtin::Log => {
                self.log_window = match self.log_window.take() {
                    Some(_) => None,
                    None => Some(log::Window::default()),
                }
            }
            Builtin::Versions => {
                self.show_versions = !self.show_versions;
                self.versions_stale = true;
            }
            Builtin::ReloadConfig => {
                self.plugins.reload();
                let loaded = self.scripts.load(&mut self.editor, &self.themes[self.theme], self.path.as_deref());
                self.lua_result(loaded);
                self.load_config(false);
                self.watch.forget_own_writes();
                self.notice = Some(("Config reloaded".into(), now));
            }
            Builtin::OpenConfigFolder | Builtin::OpenAdaptersFolder => {
                settings::write_examples();
                let dir = if b == Builtin::OpenAdaptersFolder {
                    adapters::adapters_dir()
                } else {
                    settings::config_dir()
                };
                let _ = std::fs::create_dir_all(&dir);
                self.open_external(dir.as_os_str());
            }
            Builtin::Adapters => self.show_adapters = true,
            Builtin::LinkFile => self.link_file(now),
        }
    }

    fn find_step(&mut self, forward: bool) {
        self.find.refresh(&self.editor.doc, self.editor.revision);
        if let Some((a, b)) = self.find.step(self.editor.cursor(), forward, self.editor.selection_range()) {
            self.editor.select(a, b);
        }
    }

    fn find_action(&mut self, action: find::Action, now: f64) {
        use find::Action;
        match action {
            Action::Next => self.find_step(true),
            Action::Previous => self.find_step(false),
            Action::ReplaceOne => {
                // Replace the selected match, then move to the next one.
                let selected = self.editor.selection_range();
                if let Some(m) = selected.filter(|s| self.find.matches.contains(s)) {
                    let text = self.find.replacement.clone();
                    self.editor.replace_ranges(&[m], &text, &self.themes[self.theme]);
                }
                self.find_step(true);
            }
            Action::ReplaceAll => {
                self.find.refresh(&self.editor.doc, self.editor.revision);
                let matches = self.find.matches.clone();
                let text = self.find.replacement.clone();
                self.editor.replace_ranges(&matches, &text, &self.themes[self.theme]);
                let n = matches.len();
                self.notice = Some((format!("Replaced {n} {}", if n == 1 { "match" } else { "matches" }), now));
            }
            Action::Close => {
                self.find.open = false;
                self.find_focused = false;
            }
        }
    }

    /// Opens a folder in the file browser or a link in the web browser
    /// (text9000 itself doesn't connect anywhere).
    fn open_external(&mut self, target: &std::ffi::OsStr) {
        if let Err(e) = platform::open(target) {
            self.error = Some(format!("Couldn't open it: {e}"));
        }
    }

    /// Re-reads installed adapters and rebuilds shortcuts and menus, so
    /// changes take effect without restarting.
    fn reload_adapters(&mut self) {
        self.adapters.sync(adapters::discover());
        self.rebuild_keys();
    }

    /// Documents and folio:// links the system asked text9000 to open.
    fn open_incoming(&mut self, now: f64) {
        for item in links::take() {
            match &item {
                links::Incoming::File(path) => self.open_file(path.clone(), now),
                links::Incoming::Link { kind, .. } => match self.adapters.link_handler(kind) {
                    Some((adapter, command)) => {
                        if let Err(e) = self.adapters.run_command_with(&adapter, &command, item.url().as_deref()) {
                            self.error = Some(e);
                        }
                    }
                    None if kind == "join" => {
                        self.error = Some(
                            "This is a Folio Link invite. Add the Folio Link adapter (text9000 → Adapters… → Browse Adapters) \
                             and open the link again."
                                .into(),
                        )
                    }
                    None => self.error = Some(format!("Nothing installed opens folio://{kind} links")),
                },
            }
        }
    }

    /// Files dropped on the window: an adapter's zip or folder is added, a
    /// document is opened.
    fn dropped_files(&mut self, ctx: &egui::Context, now: f64) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for path in dropped.into_iter().filter(|p| !p.as_os_str().is_empty()) {
            self.open_file(path, now);
        }
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let colors = &self.themes[self.theme].colors;
            let (bg, fg) = (color(&colors.background), color(&colors.status));
            let rect = ctx.content_rect().shrink(16.0);
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop")));
            painter.rect_filled(rect, 12.0, bg.gamma_multiply(0.9));
            painter.rect_stroke(rect, 12.0, egui::Stroke::new(2.0, fg), egui::StrokeKind::Inside);
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Drop a document to open it, an adapter's .zip to add it, or media to open it in its 9000 app",
                FontId::proportional(18.0),
                fg,
            );
        }
    }

    fn adapter_window_action(&mut self, action: adapter_window::Action, now: f64) {
        use adapter_window::Action;
        match action {
            Action::AddZip | Action::AddFolder => {
                let dialog = FileDialog::new().set_title("Add Adapter");
                let picked = if matches!(action, Action::AddZip) {
                    dialog.add_filter("Adapter", &["zip"]).pick_file()
                } else {
                    dialog.pick_folder()
                };
                if let Some(source) = picked {
                    self.add_adapter(&source, now);
                }
            }
            Action::Install(path) => self.add_adapter(&path, now),
            Action::GetAdapters => self.open_external(adapter_window::GET_ADAPTERS_URL.as_ref()),
            Action::Open(url) => self.open_external(url.as_ref()),
            Action::ShowFolder => {
                let dir = adapters::adapters_dir();
                let _ = std::fs::create_dir_all(&dir);
                self.open_external(dir.as_os_str());
            }
            Action::Run(command) => self.run(command, now),
            Action::Remove(i) => self.remove_adapter(i, now),
            Action::Close => self.show_adapters = false,
        }
    }

    fn add_adapter(&mut self, source: &std::path::Path, now: f64) {
        let unpacked = match adapters::unpack(source) {
            Ok(u) => u,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        match adapters::find_manifest(&unpacked.root, &["plugin.toml"]) {
            Ok(Some(dir)) => return self.add_plugin(&dir, now),
            Ok(None) => {}
            Err(e) => {
                self.error = Some(e);
                return;
            }
        }
        let staged = match adapters::stage_unpacked(unpacked) {
            Ok(s) => s,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let m = &staged.manifest;
        let by = if m.author.is_empty() { String::new() } else { format!(" by {}", m.author) };
        let mut text = format!("Add “{}” {}{by}?", m.name, m.version);
        if !m.description.is_empty() {
            text += &format!("\n\n{}", m.description);
        }
        if !m.permissions.is_empty() {
            text += "\n\nIt will be able to:";
            for p in &m.permissions {
                text += &format!("\n•  {}", adapters::describe(p));
            }
        }
        if self.adapters.index_of(&m.name).is_some() {
            text += &format!("\n\nThis replaces the copy you have now (it goes to {}).", platform::TRASH);
        }
        text += "\n\nAdapters are separate programs. Only add ones you trust.";
        let ok = dialog::ask("Add adapter?", &text, MessageButtons::OkCancel) == MessageDialogResult::Ok;
        if !ok {
            return;
        }
        if let Some(i) = self.adapters.index_of(&m.name) {
            self.adapters.stop(i);
        }
        match adapters::install(&staged) {
            Ok(_) => {
                self.notice = Some((format!("Added {}", staged.manifest.name), now));
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
        self.reload_adapters();
    }

    /// Installs a WebAssembly plugin after showing what it can do.
    fn add_plugin(&mut self, dir: &std::path::Path, now: f64) {
        let d = match plugins::Plugins::describe(dir) {
            Ok(d) => d,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let by = if d.author.is_empty() { String::new() } else { format!(" by {}", d.author) };
        let mut text = format!("Add the plugin “{}” {}{by}?", d.name, d.version);
        if !d.description.is_empty() {
            text += &format!("\n\n{}", d.description);
        }
        if d.can.is_empty() {
            text += "\n\nIt asks for nothing: it can't see your document, files or the network.";
        } else {
            text += "\n\nIt will be able to:";
            for c in &d.can {
                text += &format!("\n•  {c}");
            }
            text += "\n\nPlugins run in a sandbox: they can't do anything beyond this.";
        }
        let ok = dialog::ask("Add plugin?", &text, MessageButtons::OkCancel) == MessageDialogResult::Ok;
        if !ok {
            return;
        }
        match self.plugins.install(dir) {
            Ok(()) => {
                self.notice = Some((format!("Added {}", d.name), now));
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn remove_adapter(&mut self, i: usize, now: f64) {
        let name = self.adapters.all[i].manifest.name.clone();
        let text = format!("Remove “{name}”? It moves to {}, and text9000 forgets that you allowed it.", platform::TRASH);
        let ok = dialog::ask("Remove adapter?", &text, MessageButtons::OkCancel) == MessageDialogResult::Ok;
        if !ok {
            return;
        }
        self.adapters.stop(i);
        match adapters::uninstall(&self.adapters.all[i]) {
            Ok(()) => self.notice = Some((format!("Removed {name}"), now)),
            Err(e) => self.error = Some(e),
        }
        self.reload_adapters();
    }

    fn panel_action(&mut self, action: panels::Action, now: f64) {
        use panels::Action;
        match action {
            Action::Goto(para) => return self.editor.goto(para),
            Action::RestoreVersion(file) => return self.restore_version(&file),
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
        let plugin_statuses = self.plugins.status_items();
        if !plugin_statuses.is_empty() {
            f.insert("plugin_status".into(), plugin_statuses.join("   "));
        }
        let statuses: Vec<&str> = self.adapters.status_items().chain(plugin_statuses.iter().map(String::as_str)).collect();
        if !statuses.is_empty() {
            f.insert("adapter_status".into(), statuses.join("   "));
            // The older name for the same field.
            f.insert("component_status".into(), statuses.join("   "));
        }
        for (_, c) in self.adapters.installed() {
            if let Some(s) = &c.status {
                f.insert(format!("adapter:{}", c.manifest.name.to_lowercase()), s.clone());
                f.insert(format!("component:{}", c.manifest.name.to_lowercase()), s.clone());
            }
        }
        f
    }

    // ---- Adapters -------------------------------------------------------------

    /// Talks to running adapters: answers their requests and tells them
    /// when the document changes.
    fn service_adapters(&mut self, now: f64) {
        // Adapter panels on screen need their adapter running.
        if self.settings.show_panels {
            let wanted: Vec<usize> = self
                .settings
                .panels
                .iter()
                .filter(|p| p.kind == PanelKind::Adapter)
                .filter_map(|p| self.adapters.index_of(p.adapter.as_deref()?))
                .collect();
            for i in wanted {
                if let Err(e) = self.adapters.ensure_running(i) {
                    self.error = Some(e);
                }
            }
        }

        let (calls, errors) = self.adapters.poll();
        if let Some(e) = errors.into_iter().last() {
            self.error = Some(e);
        }
        for call in calls {
            let adapter = call.adapter;
            let id = call.id.clone();
            if call.method == "ui.prompt" {
                if let Some(id) = id {
                    let param = |key: &str| call.params.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
                    if self.prompt.is_some() {
                        self.adapters.reply(adapter, id, Err("another question is already open".into()));
                    } else {
                        self.prompt = Some(Prompt {
                            adapter,
                            id,
                            title: param("title"),
                            message: param("message"),
                            text: param("default"),
                            focused: false,
                        });
                    }
                }
                continue;
            }
            if call.method == "doc.print_model" {
                match (id, self.adapters.has_permission(adapter, adapters::READ)) {
                    (Some(id), true) => self.pending_prints.push((adapter, id)),
                    (Some(id), false) => self.adapters.reply(adapter, id, Err(needs_permission(adapters::READ))),
                    (None, _) => {}
                }
                continue;
            }
            let result = self.answer(call, now);
            match id {
                Some(id) => self.adapters.reply(adapter, id, result),
                None => {
                    if let Err(e) = result {
                        eprintln!("text9000: adapter message failed: {e}");
                    }
                }
            }
        }

        // Sharing adapters get the whole document on every change, right away,
        // and when the caret moves (so others see where you are).
        let fresh = std::mem::take(&mut self.adapters.just_started);
        let state = (self.editor.revision, self.editor.cursor(), self.editor.anchor());
        if self.adapters.wants("snapshot") && (fresh || self.sent_snapshot != Some(state)) {
            let doc: Value = serde_json::from_str(&self.editor.doc.to_json()).unwrap_or_default();
            let at = |p: Pos| json!({ "para": p.para, "ch": p.ch });
            self.adapters.emit(
                "snapshot",
                json!({
                    "revision": state.0,
                    "paragraphs": doc["paragraphs"],
                    "cursor": at(state.1),
                    "anchor": at(state.2),
                }),
            );
            self.sent_snapshot = Some(state);
        }
        self.editor.peers = self.adapters.peers.clone();

        // Typing changes the document constantly, so send at most ~3 updates a
        // second. An adapter that just started gets the current state right away.
        let (sent, at) = self.sent_revision;
        if fresh || (self.editor.revision != sent && now - at > 0.3) {
            let doc = &self.editor.doc;
            self.adapters.emit(
                "changed",
                json!({ "words": doc.word_count(), "paragraphs": doc.paragraphs.len() }),
            );
            self.sent_revision = (self.editor.revision, now);
        }
    }

    /// What a bug report needs to know about this text9000, for Copy Debug
    /// Info. Never any document text.
    fn debug_info(&self) -> String {
        let mut out = vec![
            format!("{} {}", plugins::EDITION, plugins::version()),
            format!("System: {} {}", std::env::consts::OS, std::env::consts::ARCH),
            format!("Config folder: {}", settings::config_dir().display()),
            format!("Theme: {}", self.theme().name),
        ];
        let mb = |b: u64| format!("{:.0} MB", b as f64 / 1_048_576.0);
        if let Some((now, peak)) = platform::memory_use() {
            let peak = peak.map(|p| format!(" (most so far: {})", mb(p))).unwrap_or_default();
            out.push(format!("Memory: {}{peak}", mb(now)));
        }
        let (size, ppp) = self.ctx.input(|i| (i.viewport_rect().size(), i.pixels_per_point));
        out.push(format!(
            "Window: {:.0}x{:.0} pixels at {ppp}x, zoom {:.0}%",
            size.x * ppp,
            size.y * ppp,
            self.editor.zoom * 100.0
        ));
        let adapters: Vec<String> = self
            .adapters
            .installed()
            .map(|(_, a)| format!("{} {}", a.manifest.name, a.manifest.version))
            .collect();
        out.push(format!("Adapters: {}", if adapters.is_empty() { "none".into() } else { adapters.join(", ") }));
        let plugins = self.plugins.loaded();
        out.push(format!("Plugins: {}", if plugins.is_empty() { "none".into() } else { plugins.join(", ") }));
        if let Some(e) = &self.error {
            out.push(format!("Showing error: {e}"));
        }
        let problems = log::recent_problems(20);
        out.push(if problems.is_empty() { "Recent problems: none".into() } else { format!("Recent problems:\n{}", problems.join("\n")) });
        // Paths show ~ for the home folder: people paste this into public issues.
        platform::hide_home(&out.join("\n"))
    }

    /// Opens a link from the document. Only web and mail links: a shared
    /// document shouldn't be able to start programs or open local files.
    /// A link to a video, sound, photo or 9000 project opens in its sister
    /// app; nothing else local is opened.
    fn open_link(&mut self, url: &str, now: f64) {
        let lower = url.to_lowercase();
        if ["https://", "http://", "mailto:"].iter().any(|s| lower.starts_with(s)) {
            self.open_external(url.as_ref());
            return;
        }
        let target = series::link_target(url, self.path.as_deref());
        match target.as_deref().and_then(|p| Some((series::for_file(p)?, p))) {
            Some((sibling, path)) if path.exists() => self.hand_off(sibling, Some(path), now),
            Some((_, path)) => self.error = Some(format!("The linked file isn't there: {}", path.display())),
            None => {
                self.notice = Some((
                    format!("text9000 opens web and mail links, and video, sound and photos in the 9000 apps, not {url}"),
                    now,
                ))
            }
        }
    }

    /// Links the selection (or, with none, the file's name, typed at the
    /// caret) to a file chosen in a dialog. Clicking the link with Cmd (Ctrl)
    /// opens it in its 9000 app. The link is relative to the document's
    /// folder when the file is inside it, so they can move together.
    fn link_file(&mut self, now: f64) {
        let mut dialog = FileDialog::new().set_title("Link to a File");
        if let Some(dir) = self.path.as_deref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(file) = dialog.pick_file() else { return };
        let url = series::link_for(&file, self.path.as_deref());
        let range = match self.editor.selection_range() {
            Some(range) => range,
            None => {
                let start = self.editor.cursor();
                let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.editor.insert_at_cursor(&name, &self.themes[self.theme]);
                (start, self.editor.cursor())
            }
        };
        match self.editor.set_link(Some(&url), Some(range)) {
            Ok(()) if series::for_file(&file).is_none() => {
                self.notice = Some(("Linked. Only videos, sound, photos and 9000 projects open from a link".into(), now))
            }
            Ok(()) => {}
            Err(e) => self.error = Some(e),
        }
    }

    /// Opens a 9000-series app, with a file if given.
    fn hand_off(&mut self, sibling: &series::Sibling, file: Option<&std::path::Path>, now: f64) {
        match series::open(sibling, file, self.settings.apps.get(sibling.name).map(String::as_str)) {
            Ok(()) => {
                let what = file.and_then(|f| f.file_name()).map(|n| format!("{} in ", n.to_string_lossy())).unwrap_or_default();
                self.notice = Some((format!("Opened {what}{}", sibling.name), now));
                log::push("text9000", log::Level::Info, format!("Opened {what}{}", sibling.name));
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Opens whatever file was asked for: an add-on is installed, a video,
    /// sound or photo goes to its 9000 app, and anything else is opened as a
    /// document (after asking about unsaved changes).
    fn open_file(&mut self, path: PathBuf, now: f64) {
        let is_addon = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) || path.is_dir();
        if is_addon {
            self.add_adapter(&path, now);
        } else if let Some(sibling) = series::for_file(&path).filter(|_| !series::is_document(&path)) {
            self.hand_off(sibling, Some(&path), now);
        } else if self.confirm_discard() {
            self.open_path(path);
        }
    }

    /// The window for an adapter's `ui.prompt` question; answers it on OK
    /// (with the text) or Cancel (with null).
    fn prompt_window(&mut self, ctx: &egui::Context) {
        let Some(prompt) = &mut self.prompt else { return };
        let mut answer: Option<Value> = None;
        egui::Window::new(if prompt.title.is_empty() { series::NAME } else { prompt.title.as_str() })
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 120.0))
            .show(ctx, |ui| {
                if !prompt.message.is_empty() {
                    ui.label(&prompt.message);
                }
                let field = ui.add(egui::TextEdit::singleline(&mut prompt.text).desired_width(360.0));
                if !prompt.focused {
                    field.request_focus();
                    prompt.focused = true;
                }
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() || enter {
                        answer = Some(json!(prompt.text));
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        answer = Some(Value::Null);
                    }
                });
            });
        if let Some(answer) = answer {
            let p = self.prompt.take().expect("open");
            self.adapters.reply(p.adapter, p.id, Ok(answer));
        }
    }

    /// Keeps plugins up to date with the document and applies what they send.
    fn service_plugins(&mut self, now: f64) {
        let theme = self.themes[self.theme].name.clone();
        for effect in self.plugins.update(&self.editor, self.path.as_deref(), &theme) {
            match effect {
                plugins::Effect::Insert { plugin, revision, text } => {
                    // Made against an older document: the text it meant to
                    // replace may have moved, so it's dropped, not misplaced.
                    let current = self.editor.revision;
                    if revision == current || self.plugin_edit == Some((revision, current)) {
                        self.editor.insert_at_cursor(&text, &self.themes[self.theme]);
                        self.plugin_edit = Some((revision, self.editor.revision));
                    } else {
                        self.notice = Some((format!("{plugin}'s edit was skipped because the document changed"), now));
                    }
                }
                plugins::Effect::Replace {
                    plugin,
                    revision,
                    start,
                    end,
                    text,
                } => {
                    let current = self.editor.revision;
                    let fresh = revision == current || self.plugin_edit == Some((revision, current));
                    let doc = &self.editor.doc;
                    let valid = |p: doc::Pos| p.para < doc.paragraphs.len() && p.ch <= doc.paragraphs[p.para].len();
                    if fresh && valid(start) && valid(end) {
                        self.editor.replace_ranges(&[(start, end)], &text, &self.themes[self.theme]);
                        self.plugin_edit = Some((revision, self.editor.revision));
                    } else if !fresh {
                        self.notice = Some((format!("{plugin}'s edit was skipped because the document changed"), now));
                    } else {
                        log::push(&plugin, log::Level::Warn, "replace-range was outside the document");
                    }
                }
                plugins::Effect::Notify(message) => {
                    log::push("Plugins", log::Level::Info, &message);
                    self.notice = Some((message, now));
                }
                plugins::Effect::Problem(message) => self.error = Some(message),
                plugins::Effect::CommandsChanged => self.rebuild_keys(),
            }
        }
    }

    fn answer(&mut self, call: Call, now: f64) -> Result<Value, String> {
        let needs = |permission: &str| {
            if self.adapters.has_permission(call.adapter, permission) {
                Ok(())
            } else {
                Err(needs_permission(permission))
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
                needs(adapters::READ)?;
                let mut doc: Value = serde_json::from_str(&self.editor.doc.to_json()).map_err(|e| e.to_string())?;
                doc["path"] = json!(self.path);
                doc["theme"] = json!(self.theme().name);
                let at = |p: Pos| json!({ "para": p.para, "ch": p.ch });
                doc["cursor"] = at(self.editor.cursor());
                doc["anchor"] = at(self.editor.anchor());
                doc["selection"] = self
                    .editor
                    .selection_range()
                    .map_or(Value::Null, |(a, b)| json!({ "start": at(a), "end": at(b) }));
                Ok(doc)
            }
            "doc.set_list" => {
                needs(adapters::EDIT)?;
                let list = match call.params.get("list") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(serde_json::from_value::<doc::List>(v.clone()).map_err(|e| format!("list: {e}"))?),
                };
                let para = |key: &str| call.params.get(key).and_then(Value::as_u64).map(|n| n as usize);
                let range = para("from").map(|from| (from, para("to").unwrap_or(from)));
                self.editor.set_list(list, range);
                Ok(Value::Null)
            }
            "doc.set_link" => {
                needs(adapters::EDIT)?;
                let url = call.params.get("url").and_then(Value::as_str).filter(|u| !u.is_empty());
                let pos = |key: &str| {
                    let v = call.params.get(key)?;
                    Some(Pos::new(v.get("para")?.as_u64()? as usize, v.get("ch")?.as_u64()? as usize))
                };
                let range = pos("start").zip(pos("end"));
                self.editor.set_link(url, range)?;
                Ok(Value::Null)
            }
            "doc.text" => {
                needs(adapters::READ)?;
                let doc = &self.editor.doc;
                Ok(json!({ "text": doc.text(Pos::default(), doc.end()) }))
            }
            "doc.insert" => {
                needs(adapters::EDIT)?;
                let text = text_param()?;
                self.editor.insert_at_cursor(&text, &self.themes[self.theme]);
                Ok(Value::Null)
            }
            "doc.replace" => {
                needs(adapters::EDIT)?;
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
            "doc.merge" => {
                needs(adapters::EDIT)?;
                Ok(json!({ "applied": self.editor.apply_merge(&call.params)? }))
            }
            "ui.confirm" => {
                let param = |key: &str| call.params.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
                let yes = dialog::ask(&param("title"), &param("message"), MessageButtons::YesNo) == MessageDialogResult::Yes;
                Ok(json!(yes))
            }
            "ui.save_dialog" => {
                let param = |key: &str| call.params.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
                let mut dialog = FileDialog::new().set_title(param("title")).set_file_name(param("file_name"));
                let extension = param("extension");
                if !extension.is_empty() {
                    dialog = dialog.add_filter(extension.to_uppercase(), &[extension.as_str()]);
                }
                if let Some(dir) = self.path.as_deref().and_then(|p| p.parent()) {
                    dialog = dialog.set_directory(dir);
                }
                Ok(dialog.save_file().map_or(Value::Null, |p| json!(p)))
            }
            other => Err(format!("unknown method \"{other}\"")),
        }
    }

    // ---- Drawing --------------------------------------------------------------

    /// Draws the top strip in the theme's colors: on macOS the title, under
    /// the transparent titlebar, which also moves the window; on Windows and
    /// Linux, below the system's own titlebar, the menus.
    fn titlebar(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let pal = series::PALETTE;
        let painter = ui.painter().clone();
        painter.rect_filled(rect, 0.0, pal.header);
        painter.rect_filled(egui::Rect::from_x_y_ranges(rect.x_range(), rect.bottom() - 1.0..=rect.bottom()), 0.0, pal.line);
        if cfg!(target_os = "macos") {
            // The mark and name after the traffic lights, the document in
            // the middle, as across the 9000 series.
            series::wordmark(&painter, egui::pos2(rect.left() + 84.0, rect.center().y), &pal);
            painter.text(rect.center(), Align2::CENTER_CENTER, &self.title, FontId::proportional(12.0), pal.weak);

            let response = ui.interact(rect, egui::Id::unique("titlebar"), Sense::click_and_drag());
            if response.double_clicked() {
                let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!maximized));
            } else if response.drag_started() {
                ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
            }
        } else {
            // Menus on the left, the mark and name at the right end.
            if let Some(menu) = &mut self.menu {
                menu.show(ui, rect, &pal);
            }
            let galley_w = 24.0 + 30.0 + 34.0;
            series::wordmark(&painter, egui::pos2(rect.right() - galley_w - 12.0, rect.center().y), &pal);
        }

        // The system's own parts (traffic lights, the title bar) match the
        // graphite strip.
        if self.appearance_for.is_none() {
            ui.ctx().send_viewport_cmd(ViewportCommand::SetTheme(SystemTheme::Dark));
            ui.ctx().set_visuals(pal.visuals());
            self.appearance_for = Some(pal.header);
        }
    }

    fn versions_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let mut restore = None;
        egui::Window::new("Versions").open(&mut open).default_width(320.0).show(ctx, |ui| {
            if self.editor.doc.id.is_none() || self.versions.is_empty() {
                ui.label("text9000 keeps a version each time you save this document.");
                return;
            }
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                for v in &self.versions {
                    ui.horizontal(|ui| {
                        ui.strong(&v.when);
                        ui.label(format!("{} words", v.words));
                        if ui.button("Restore").clicked() {
                            restore = Some(v.path.clone());
                        }
                    });
                    ui.weak(&v.preview);
                    ui.separator();
                }
            });
        });
        self.show_versions = open;
        if let Some(file) = restore {
            self.restore_version(&file);
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
            let unbound: Vec<String> = self
                .scripts
                .names()
                .into_iter()
                .filter(|name| self.keymap.chord_for(&Command::Lua(name.clone())).is_none())
                .map(|name| format!("lua {name}"))
                .collect();
            if !unbound.is_empty() {
                text += &format!("\nNo key yet (Scripts menu): {}\n", unbound.join(", "));
            }
            text += "\nRemap any of these in keys.toml (text9000 → Open Config Folder)";
            let galley = painter.layout_no_wrap(text, FontId::monospace(12.0), egui::Color32::WHITE);
            let box_rect = egui::Rect::from_center_size(rect.center(), galley.size() + egui::vec2(36.0, 30.0));
            painter.rect_filled(box_rect, 8.0, egui::Color32::from_black_alpha(225));
            painter.galley(box_rect.min + egui::vec2(18.0, 15.0), galley, egui::Color32::WHITE);
        }
    }
}

impl shell::App for Text9000 {
    /// Caps egui's textures at about 2048 pixels a side (see
    /// `trim_text_texture` for why it's sometimes 2047). The text texture
    /// holds every letter drawn, at every size, and tops out at 16 MB; egui
    /// clears it when it's nearly full.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        raw_input.max_texture_side = Some(raw_input.max_texture_side.unwrap_or(self.texture_side).min(self.texture_side));
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        if let Some(fonts) = self.next_fonts.take() {
            self.fonts = fonts;
        }
        self.trim_text_texture(&ctx, now);

        if ctx.input(|i| i.viewport().close_requested()) {
            if self.confirm_discard() {
                // A clean quit: nothing to recover next time.
                safety::clear_recovery();
            } else {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            }
        }
        if self.watch.changed(now) {
            self.load_config(false);
        }
        // Check the config files again even when idle, so edits show up.
        ctx.request_repaint_after(std::time::Duration::from_secs(1));

        let mut commands = self.menu.as_mut().map(MenuBar::poll).unwrap_or_default();
        // While typing in the find bar, only find commands use the keyboard.
        let typing_in_find = self.find_focused;
        commands.extend(self.keymap.take_commands(&ctx, |c| {
            !typing_in_find
                || matches!(c, Command::Builtin(Builtin::Find | Builtin::FindNext | Builtin::FindPrevious | Builtin::Replace))
        }));
        for command in commands {
            self.run(command, now);
        }
        self.dropped_files(&ctx, now);
        self.open_incoming(now);
        self.track_window(&ctx, now);
        self.service_adapters(now);
        self.service_plugins(now);
        self.safety_net(now);
        self.refresh_versions();

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
                host: &self.adapters,
                plugins: &self.plugins,
                versions: &self.versions,
            };
            let actions = panels::show(&mut body_ui, &self.settings.panels, &cx);
            for action in actions {
                self.panel_action(action, now);
            }
        }

        if self.find.open {
            self.find.refresh(&self.editor.doc, self.editor.revision);
            let selection = self.editor.selection_range();
            let (actions, focused) = self.find.show(&mut body_ui, &self.themes[self.theme], selection);
            self.find_focused = focused;
            for action in actions {
                self.find_action(action, now);
            }
            if !self.find_focused && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.find_action(find::Action::Close, now);
            }
        } else {
            self.find_focused = false;
        }
        self.editor.highlights = if self.find.open { self.find.matches.clone() } else { Vec::new() };
        self.editor.keyboard = !self.find_focused && self.prompt.is_none();
        self.editor.smart_quotes = self.settings.smart_quotes;
        self.editor.smart_dashes = self.settings.smart_dashes;

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
        if !self.pending_prints.is_empty() {
            let model = self.editor.print_model(&page_ui, &self.themes[self.theme], &self.fonts, &page_text);
            let title = values.get("file").cloned().unwrap_or_default();
            let json = pdf::to_json(&model, &title);
            for (adapter, id) in std::mem::take(&mut self.pending_prints) {
                self.adapters.reply(adapter, id, json.clone());
            }
        }
        self.editor.show(&mut page_ui, &self.themes[self.theme], &self.fonts, &page_text, self.speller.as_mut());
        if let Some(url) = self.editor.open_link.take() {
            self.open_link(&url, now);
        }
        self.prompt_window(&ctx);

        if self.log_window.is_some() {
            let info = self.debug_info();
            if let Some(w) = &mut self.log_window
                && !w.show(&ctx, &|| info.clone())
            {
                self.log_window = None;
            }
        }
        if self.error.is_some() && self.error != self.logged_error {
            log::push("text9000", log::Level::Error, self.error.clone().unwrap_or_default());
        }
        self.logged_error = self.error.clone();
        if self.show_versions {
            self.versions_window(&ctx);
        }
        if self.show_adapters {
            let actions = adapter_window::show(&ctx, &self.adapters, &self.keymap);
            for action in actions {
                self.adapter_window_action(action, now);
            }
        }

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

/// A question from an adapter (`ui.prompt`).
struct Prompt {
    adapter: usize,
    id: Value,
    title: String,
    message: String,
    text: String,
    /// The text field has been given the keyboard once.
    focused: bool,
}

fn needs_permission(permission: &str) -> String {
    format!("this needs the \"{permission}\" permission in adapter.toml")
}

/// Local date (YYYY-MM-DD) and time (HH:MM).
fn local_date_time() -> (String, String) {
    let t = platform::local_time();
    (format!("{:04}-{:02}-{:02}", t.year, t.month, t.day), format!("{:02}:{:02}", t.hour, t.minute))
}

#[cfg(unix)]
#[global_allocator]
static ALLOCATOR: alloc::Allocator = alloc::Allocator;

const GUIDE_URL: &str = "https://github.com/solacecasino/text9000#readme";
const REPORT_URL: &str = "https://github.com/solacecasino/text9000/issues/new?template=bug.yml";

fn main() -> Result<(), String> {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        // Release builds on Windows have no console of their own; print to
        // the terminal text9000 was started from. (Redirected output already works.)
        #[cfg(windows)]
        // SAFETY: attaching to the parent's console has no preconditions.
        unsafe {
            let _ = windows::Win32::System::Console::AttachConsole(windows::Win32::System::Console::ATTACH_PARENT_PROCESS);
        }
        // Also how CI checks that each download starts.
        println!("{} {}", plugins::EDITION, plugins::version());
        return Ok(());
    }
    if let Some(problem) = platform::missing_system_library() {
        eprintln!("text9000: {problem}");
        // Started from a menu there's no terminal to read; try a dialog.
        let _ = std::process::Command::new("zenity").args(["--error", "--title=text9000", "--text", &problem]).status();
        std::process::exit(1);
    }
    // Folio users keep everything: settings, keys, themes, adapters, versions.
    if series::env("CONFIG_DIR").is_none()
        && let Some(from) = platform::adopt_folio_config(&platform::default_config_dir(), &platform::folio_config_dir())
    {
        log::push("text9000", log::Level::Info, format!("Copied Folio's settings from {}", platform::hide_home(&from.display().to_string())));
        let _ = platform::ADOPTED.set(from);
    }
    log::catch_crashes();
    links::listen_early();
    // A document to open, or a folio:// link (queued until the app is up).
    let path = match std::env::args_os().nth(1) {
        Some(arg) if ["folio://", "text9000://"].iter().any(|s| arg.to_string_lossy().starts_with(s)) => {
            links::deliver(links::parse(&arg.to_string_lossy()));
            None
        }
        arg => arg.map(PathBuf::from),
    };
    let icon = shell::icon_from_png(ICON_PNG).expect("bundled icon is a valid PNG");
    let state = state::State::load();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(series::NAME)
        .with_app_id(series::NAME)
        .with_icon(icon.clone())
        .with_inner_size([1000.0, 1100.0]);
    if let Some(w) = state.window {
        viewport = viewport.with_inner_size([w.width, w.height]).with_position([w.x, w.y]);
    }
    let viewport = viewport
        // Draw under a transparent titlebar so the theme colors the whole window.
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false);
    shell::run(viewport, move |ctx| Text9000::new(ctx, path, icon, state))
}
