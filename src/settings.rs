//! The config folder and `settings.toml`. Folio has no settings window: every
//! preference is a line in a text file you can edit, copy and share.
//!
//! Everything lives in one folder (`~/Library/Application Support/Folio` on
//! macOS, `%APPDATA%\Folio` on Windows, `~/.config/folio` on Linux):
//! `settings.toml`, `keys.toml`, `themes/`, `adapters/`. Sharing that one
//! folder shares a whole setup.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;

use serde::Deserialize;

/// `FOLIO_CONFIG_DIR` points Folio at another folder, e.g. a portable setup
/// on a USB stick.
pub fn config_dir() -> PathBuf {
    std::env::var_os("FOLIO_CONFIG_DIR")
        .filter(|v| !v.is_empty())
        .map_or_else(crate::platform::default_config_dir, PathBuf::from)
}

pub fn settings_path() -> PathBuf {
    config_dir().join("settings.toml")
}

pub fn keys_path() -> PathBuf {
    config_dir().join("keys.toml")
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Theme to start with. Documents saved with a theme still open in it.
    pub theme: Option<String>,
    pub zoom: f32,
    pub show_status: bool,
    pub show_panels: bool,
    /// Status line, in the formatting language (see docs/customizing.md).
    pub status: String,
    /// Window title, in the formatting language.
    pub title: String,
    pub panels: Vec<PanelSpec>,
    /// Save named documents a moment after you stop typing.
    pub autosave: bool,
    /// Snapshots kept per document in the config folder's versions/ (0 = off).
    pub versions: usize,
    /// macOS spell checking with red underlines.
    pub spellcheck: bool,
    /// A language code like "en" or "fr"; empty uses the system language.
    pub spell_language: String,
    /// Curly quotes as you type.
    pub smart_quotes: bool,
    /// -- becomes — and ... becomes …
    pub smart_dashes: bool,
}

pub const DEFAULT_STATUS: &str =
    "[%adapter_status%   ]%theme%   %pages% $plural(%pages%,page)   %words% $plural(%words%,word)   %zoom%'%'";
pub const DEFAULT_TITLE: &str = "%file%$if(%dirty%, •,)";

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: None,
            zoom: 1.0,
            show_status: true,
            show_panels: true,
            status: DEFAULT_STATUS.into(),
            title: DEFAULT_TITLE.into(),
            panels: Vec::new(),
            autosave: true,
            versions: 50,
            spellcheck: true,
            spell_language: String::new(),
            smart_quotes: true,
            smart_dashes: true,
        }
    }
}

const KNOWN_KEYS: &[&str] = &[
    "theme", "zoom", "show_status", "show_panels", "status", "title", "panels", "autosave", "versions",
    "spellcheck", "spell_language", "smart_quotes", "smart_dashes",
];

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct PanelSpec {
    pub kind: PanelKind,
    #[serde(default)]
    pub side: Side,
    #[serde(default = "default_panel_size")]
    pub size: f32,
    /// Heading shown at the top of the panel.
    pub title: Option<String>,
    /// For `text` panels: lines in the formatting language.
    #[serde(default)]
    pub lines: Vec<String>,
    /// For `adapter` panels: the adapter's name and panel id.
    #[serde(alias = "component")]
    pub adapter: Option<String>,
    pub id: Option<String>,
}

fn default_panel_size() -> f32 {
    220.0
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PanelKind {
    Outline,
    Text,
    /// A panel filled by an adapter ("component" is the older name).
    #[serde(alias = "component")]
    Adapter,
    Versions,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    #[default]
    Left,
    Right,
    Bottom,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
            Side::Bottom => "bottom",
        }
    }
}

/// Loads settings, returning problems to show the user instead of failing:
/// a broken file falls back to defaults so Folio always opens.
pub fn load() -> (Settings, Vec<String>) {
    let Ok(src) = std::fs::read_to_string(settings_path()) else {
        return (Settings::default(), Vec::new());
    };
    let mut problems = Vec::new();
    let settings = match toml::from_str::<Settings>(&src) {
        Ok(s) => s,
        Err(e) => {
            problems.push(format!("settings.toml: {}", e.message()));
            Settings::default()
        }
    };
    // Unknown keys are reported, not fatal, so old setups keep working.
    if let Ok(table) = src.parse::<toml::Table>() {
        for key in table.keys().filter(|k| !KNOWN_KEYS.contains(&k.as_str())) {
            problems.push(format!("settings.toml: unknown setting \"{key}\""));
        }
    }
    (settings, problems)
}

pub fn load_keys() -> (BTreeMap<String, String>, Vec<String>) {
    let Ok(src) = std::fs::read_to_string(keys_path()) else {
        return (BTreeMap::new(), Vec::new());
    };
    match toml::from_str::<BTreeMap<String, String>>(&src) {
        Ok(keys) => (keys, Vec::new()),
        Err(e) => (BTreeMap::new(), vec![format!("keys.toml: {}", e.message())]),
    }
}

/// Edits `settings.toml` in place, keeping the user's comments and layout.
fn edit(change: impl FnOnce(&mut toml_edit::DocumentMut)) -> Result<(), String> {
    let path = settings_path();
    let src = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = src.parse().map_err(|e| format!("settings.toml: {e}"))?;
    change(&mut doc);
    std::fs::create_dir_all(config_dir()).map_err(|e| e.to_string())?;
    std::fs::write(&path, doc.to_string()).map_err(|e| e.to_string())
}

/// Saves one setting, e.g. the theme after Cmd+T, so Folio remembers it.
pub fn save_value(key: &str, value: impl Into<toml_edit::Value>) -> Result<(), String> {
    let value = value.into();
    edit(|doc| doc[key] = toml_edit::value(value))
}

/// Writes the panel layout back, so the file always matches the screen.
pub fn save_panels(panels: &[PanelSpec]) -> Result<(), String> {
    edit(|doc| {
        let mut array = toml_edit::ArrayOfTables::new();
        for p in panels {
            let mut t = toml_edit::Table::new();
            let kind = match p.kind {
                PanelKind::Outline => "outline",
                PanelKind::Text => "text",
                PanelKind::Adapter => "adapter",
                PanelKind::Versions => "versions",
            };
            t["kind"] = toml_edit::value(kind);
            t["side"] = toml_edit::value(p.side.name());
            t["size"] = toml_edit::value(p.size.round() as i64);
            if let Some(title) = &p.title {
                t["title"] = toml_edit::value(title.as_str());
            }
            if !p.lines.is_empty() {
                let lines: toml_edit::Array = p.lines.iter().map(String::as_str).collect();
                t["lines"] = toml_edit::value(lines);
            }
            if let Some(c) = &p.adapter {
                t["adapter"] = toml_edit::value(c.as_str());
            }
            if let Some(id) = &p.id {
                t["id"] = toml_edit::value(id.as_str());
            }
            array.push(t);
        }
        if panels.is_empty() {
            doc.remove("panels");
        } else {
            doc["panels"] = toml_edit::Item::ArrayOfTables(array);
        }
    })
}

/// Creates the config folder with commented example files, once, so people
/// can discover what's customizable by opening them.
pub fn write_examples() {
    let dir = config_dir();
    let _ = std::fs::create_dir_all(dir.join("themes"));
    let _ = std::fs::create_dir_all(dir.join("adapters"));
    for (path, text) in [(settings_path(), EXAMPLE_SETTINGS), (keys_path(), EXAMPLE_KEYS)] {
        if !path.exists() {
            let _ = std::fs::write(path, text);
        }
    }
}

const EXAMPLE_SETTINGS: &str = r#"# Folio settings. Everything here is optional; delete a line to get the default.
# Folio reloads this file as soon as you save it.
# The formatting language is described in docs/customizing.md.

# theme = "Paper"
# zoom = 1.0
# show_status = true

# Writing
# autosave = true          # save named documents 2 seconds after you stop typing
# versions = 50            # snapshots kept per document (File → Browse Versions); 0 turns it off
# spellcheck = true
# spell_language = "en"    # empty = your system language
# smart_quotes = true      # "curly" quotes as you type
# smart_dashes = true      # -- becomes — and ... becomes …

# Status line and window title. Fields look like %words%; [ ] hides a part
# when its fields are empty; $if, $plural, $upper and friends are functions.
# status = "[%adapter_status%   ]%theme%   %pages% $plural(%pages%,page)   %words% $plural(%words%,word)   %zoom%'%'"
# title = "%file%$if(%dirty%, •,)"

# Panels are off until you add some. Right-click a panel's title to move or
# close it; drag its edge to resize. Folio writes those changes back here.
#
# [[panels]]
# kind = "outline"        # clickable headings
# side = "left"           # left, right or bottom
# size = 220
#
# [[panels]]
# kind = "text"           # your own lines, in the formatting language
# side = "right"
# title = "Stats"
# lines = ["%words% words", "%chars% characters", "%reading_time% to read", "page %page% of %pages%"]
#
# [[panels]]
# kind = "versions"       # earlier versions of this document, click to restore
# side = "right"
#
# [[panels]]
# kind = "adapter"        # a panel provided by an adapter
# adapter = "Writing Tools"
# id = "goal"
"#;

const EXAMPLE_KEYS: &str = r#"# Folio key bindings: "keys" = "command". Your lines override the defaults;
# bind a key to "none" to free it. Folio reloads this file when you save it.
# Press Cmd+/ (Ctrl+/ on Windows and Linux) in Folio to see every binding.
#
# Modifiers: cmd, shift, alt (or option), ctrl. On Windows and Linux, cmd
# means Ctrl, so the same keys.toml works on every computer.
# Commands: new, open, save, save-as, quit, undo, redo, select-all, bold,
# italic, underline, next-theme, zoom-in, zoom-out, zoom-reset, toggle-status,
# toggle-panels, help, reload-config, open-config-folder,
# style <name>, adapter "<Name>" <command-id>

# "cmd+shift+h" = "style h2"
# "cmd+t" = "none"
# "cmd+alt+g" = 'adapter "Writing Tools" goal'
"#;

/// Notices when config files change on disk, checking about once a second.
pub struct Watch {
    seen: BTreeMap<PathBuf, SystemTime>,
    last_check: f64,
}

impl Watch {
    pub fn new() -> Self {
        let mut w = Self {
            seen: BTreeMap::new(),
            last_check: 0.0,
        };
        w.seen = w.scan();
        w
    }

    fn scan(&self) -> BTreeMap<PathBuf, SystemTime> {
        let mut files = vec![settings_path(), keys_path()];
        for dir in [config_dir().join("themes"), PathBuf::from("themes")] {
            if let Ok(entries) = std::fs::read_dir(dir) {
                files.extend(entries.flatten().map(|e| e.path()));
            }
        }
        files
            .into_iter()
            .filter_map(|p| Some((p.clone(), std::fs::metadata(&p).ok()?.modified().ok()?)))
            .collect()
    }

    /// True if anything changed since the last call (or `forget`).
    pub fn changed(&mut self, now: f64) -> bool {
        if now - self.last_check < 1.0 {
            return false;
        }
        self.last_check = now;
        let current = self.scan();
        let changed = current != self.seen;
        self.seen = current;
        changed
    }

    /// Call after Folio writes a config file itself, so that write doesn't
    /// count as a change.
    pub fn forget_own_writes(&mut self) {
        self.seen = self.scan();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::Format;

    #[test]
    fn shipped_examples_and_defaults_are_valid() {
        let s: Settings = toml::from_str(EXAMPLE_SETTINGS).unwrap();
        assert_eq!(s.status, DEFAULT_STATUS);
        assert!(toml::from_str::<BTreeMap<String, String>>(EXAMPLE_KEYS).unwrap().is_empty());
        Format::parse(DEFAULT_STATUS).unwrap();
        Format::parse(DEFAULT_TITLE).unwrap();

        // The commented panel examples work once uncommented.
        let uncommented: String = EXAMPLE_SETTINGS
            .lines()
            .filter(|l| l.starts_with("# [[panels]]") || l.starts_with("# kind") || l.starts_with("# side")
                || l.starts_with("# size") || l.starts_with("# title") || l.starts_with("# lines")
                || l.starts_with("# adapter") || l.starts_with("# id"))
            .map(|l| format!("{}\n", &l[2..]))
            .collect();
        let s: Settings = toml::from_str(&uncommented).unwrap();
        assert_eq!(s.panels.len(), 4);
        assert_eq!(s.panels[1].lines.len(), 4);
        for line in &s.panels[1].lines {
            Format::parse(line).unwrap();
        }
    }
}
