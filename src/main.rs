//! Folio: a document editor that starts bare and gets out of the way.
//! How documents look comes from theme files; how Folio behaves comes from
//! settings.toml and keys.toml; everything extra comes from adapters.

// Release builds on Windows are window apps, with no console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod adapter_window;
mod adapters;
mod doc;
mod editor;
mod find;
mod format;
mod keys;
mod menu;
mod panels;
mod pdf;
mod platform;
mod safety;
mod settings;
mod spell;
mod theme;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Align2, FontId, Sense, SystemTheme, ViewportCommand};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult};
use serde_json::{Value, json};

use adapters::{Call, Host};
use doc::{Document, Fmt, Pos};
use editor::Editor;
use format::Format;
use keys::{Builtin, Command, Keymap};
use menu::{MenuBar, MenuContent};
use settings::{PanelKind, Settings};
use theme::{FontMap, Theme, color};

pub const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

/// Height of the strip drawn where the macOS titlebar would be (the menus
/// on Windows and Linux).
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
    find: find::FindBar,
    /// Whether the find bar had the keyboard last frame.
    find_focused: bool,
    show_versions: bool,
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
    /// (revision, cursor, anchor) last sent in a `snapshot` event.
    sent_snapshot: Option<(u64, Pos, Pos)>,
    title: String,
    error: Option<String>,
    menu: Option<MenuBar>,
    adapters: Host,
    /// A short message from an adapter, and when it arrived.
    notice: Option<(String, f64)>,
    /// Last document revision adapters were told about, and when.
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
            show_adapters: false,
            speller: None,
            versions: Vec::new(),
            versions_stale: true,
            seen_revision: (0, 0.0),
            safety_revision: 0,
            last_auto_version: f64::NEG_INFINITY,
            pending_prints: Vec::new(),
            sent_snapshot: None,
            title: String::new(),
            error: None,
            menu: None,
            adapters: host,
            notice: None,
            sent_revision: (0, 0.0),
            appearance_for: None,
        };
        app.load_config(true);
        if let Some(p) = path {
            app.open_path(p);
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

        let language_changed = settings.spell_language != self.settings.spell_language;
        if !settings.spellcheck {
            self.speller = None;
        } else if self.speller.is_none() || language_changed || first {
            self.speller = Some(spell::Speller::new(&settings.spell_language));
        }
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
                self.versions_stale = true;
                self.adapters.emit("opened", json!({ "path": path }));
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

    /// Offers back work left behind when Folio didn't quit cleanly.
    fn offer_recovery(&mut self) {
        let Some((file, record)) = safety::orphaned_recoveries().into_iter().next() else { return };
        let restore = MessageDialog::new()
            .set_title("Restore unsaved work?")
            .set_description(record.describe())
            .set_buttons(MessageButtons::YesNo)
            .show()
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
        let confirm = MessageDialog::new()
            .set_title("Restore this version?")
            .set_description(
                "The document will be replaced with this version. You can undo it, and your current text \
                 is kept as a version the next time you save.",
            )
            .set_buttons(MessageButtons::OkCancel)
            .show();
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
            Command::Adapter(name, id) => {
                if let Err(e) = self.adapters.run_command(&name, &id) {
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
                    self.versions_stale = true;
                    safety::clear_recovery();
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
                    self.notice = Some(("No panels yet. Add [[panels]] to settings.toml (Folio → Open Config Folder)".into(), now));
                    return;
                }
                self.settings.show_panels = !self.settings.show_panels;
                let on = self.settings.show_panels;
                self.persist(|| settings::save_value("show_panels", on));
            }
            Builtin::Help => self.show_help = !self.show_help,
            Builtin::Versions => {
                self.show_versions = !self.show_versions;
                self.versions_stale = true;
            }
            Builtin::ReloadConfig => {
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
    /// (Folio itself doesn't connect anywhere).
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
            Action::GetAdapters => self.open_external(adapter_window::GET_ADAPTERS_URL.as_ref()),
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
        let staged = match adapters::stage(source) {
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
        let ok = MessageDialog::new()
            .set_title("Add adapter?")
            .set_description(text)
            .set_buttons(MessageButtons::OkCancel)
            .show()
            == MessageDialogResult::Ok;
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

    fn remove_adapter(&mut self, i: usize, now: f64) {
        let name = self.adapters.all[i].manifest.name.clone();
        let ok = MessageDialog::new()
            .set_title("Remove adapter?")
            .set_description(format!(
                "Remove “{name}”? It moves to {}, and Folio forgets that you allowed it.",
                platform::TRASH
            ))
            .set_buttons(MessageButtons::OkCancel)
            .show()
            == MessageDialogResult::Ok;
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
        let statuses: Vec<&str> = self.adapters.status_items().collect();
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
                        eprintln!("folio: adapter message failed: {e}");
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
                Ok(doc)
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
                let yes = MessageDialog::new()
                    .set_title(param("title"))
                    .set_description(param("message"))
                    .set_buttons(MessageButtons::YesNo)
                    .show()
                    == MessageDialogResult::Yes;
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
        let colors = &self.themes[self.theme].colors;
        let bg = color(&colors.background);
        if cfg!(target_os = "macos") {
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
        } else if let Some(menu) = &mut self.menu {
            menu.show(ui, rect, &self.themes[self.theme]);
        }

        // Match the system UI (traffic lights, a dark titlebar) to the theme.
        if self.appearance_for != Some(bg) {
            let [r, g, b, _] = bg.to_array();
            let luma = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
            let system = if luma < 128.0 { SystemTheme::Dark } else { SystemTheme::Light };
            ui.ctx().send_viewport_cmd(ViewportCommand::SetTheme(system));
            self.appearance_for = Some(bg);
        }
    }

    fn versions_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let mut restore = None;
        egui::Window::new("Versions").open(&mut open).default_width(320.0).show(ctx, |ui| {
            if self.editor.doc.id.is_none() || self.versions.is_empty() {
                ui.label("Folio keeps a version each time you save this document.");
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
        self.service_adapters(now);
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
        self.editor.keyboard = !self.find_focused;
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

        if self.show_versions {
            self.versions_window(&ctx);
        }
        if self.show_adapters {
            let actions = adapter_window::show(&ctx, &self.adapters, &self.themes[self.theme], &self.keymap);
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

fn needs_permission(permission: &str) -> String {
    format!("this needs the \"{permission}\" permission in adapter.toml")
}

/// Local date (YYYY-MM-DD) and time (HH:MM).
fn local_date_time() -> (String, String) {
    let t = platform::local_time();
    (format!("{:04}-{:02}-{:02}", t.year, t.month, t.day), format!("{:02}:{:02}", t.hour, t.minute))
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
