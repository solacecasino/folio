//! Named commands and the keymap.
//!
//! Every action in Folio is a command with a name (`bold`, `save`,
//! `style h1`, `adapter "Writing Tools" goal`). `keys.toml` binds key
//! combinations to command names, like mpv's input.conf, and the same table
//! drives the menu bar's shortcuts and the in-window keyboard handling.

use std::collections::BTreeMap;
use std::fmt;

use egui::{self, Key};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Builtin {
    New,
    Open,
    Save,
    SaveAs,
    Versions,
    Quit,
    Undo,
    Redo,
    SelectAll,
    Find,
    FindNext,
    FindPrevious,
    Replace,
    Bold,
    Italic,
    Underline,
    NextTheme,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleStatus,
    TogglePanels,
    Help,
    Guide,
    ReportProblem,
    ReloadConfig,
    OpenConfigFolder,
    OpenAdaptersFolder,
    Adapters,
    Log,
}

/// (command, name in keys.toml, menu label, default keys)
pub const BUILTINS: &[(Builtin, &str, &str, &[&str])] = &[
    (Builtin::New, "new", "New", &["cmd+n"]),
    (Builtin::Open, "open", "Open…", &["cmd+o"]),
    (Builtin::Save, "save", "Save", &["cmd+s"]),
    (Builtin::SaveAs, "save-as", "Save As…", &["cmd+shift+s"]),
    (Builtin::Versions, "versions", "Browse Versions…", &[]),
    (Builtin::Quit, "quit", "Quit Folio", &["cmd+q"]),
    (Builtin::Undo, "undo", "Undo", &["cmd+z"]),
    (Builtin::Redo, "redo", "Redo", &["cmd+shift+z"]),
    (Builtin::SelectAll, "select-all", "Select All", &["cmd+a"]),
    (Builtin::Find, "find", "Find…", &["cmd+f"]),
    (Builtin::FindNext, "find-next", "Find Next", &["cmd+g"]),
    (Builtin::FindPrevious, "find-previous", "Find Previous", &["cmd+shift+g"]),
    (Builtin::Replace, "replace", "Find and Replace…", &["cmd+alt+f"]),
    (Builtin::Bold, "bold", "Bold", &["cmd+b"]),
    (Builtin::Italic, "italic", "Italic", &["cmd+i"]),
    (Builtin::Underline, "underline", "Underline", &["cmd+u"]),
    (Builtin::NextTheme, "next-theme", "Next Theme", &["cmd+t"]),
    (Builtin::ZoomIn, "zoom-in", "Zoom In", &["cmd+=", "cmd++"]),
    (Builtin::ZoomOut, "zoom-out", "Zoom Out", &["cmd+-"]),
    (Builtin::ZoomReset, "zoom-reset", "Actual Size", &[]),
    (Builtin::ToggleStatus, "toggle-status", "Show Status Line", &["cmd+."]),
    (Builtin::TogglePanels, "toggle-panels", "Show Panels", &["cmd+\\"]),
    (Builtin::Help, "help", "Keyboard Shortcuts", &["cmd+/"]),
    (Builtin::Guide, "guide", "User Guide", &[]),
    (Builtin::ReportProblem, "report-problem", "Report a Problem…", &[]),
    (Builtin::Log, "log", "Log", &["cmd+alt+l"]),
    (Builtin::ReloadConfig, "reload-config", "Reload Config", &["cmd+shift+r"]),
    (Builtin::OpenConfigFolder, "open-config-folder", "Open Config Folder", &[]),
    (Builtin::OpenAdaptersFolder, "open-adapters-folder", "Open Adapters Folder", &[]),
    (Builtin::Adapters, "adapters", "Adapters…", &["cmd+shift+a"]),
];

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    Builtin(Builtin),
    /// Apply a paragraph style from the current theme.
    Style(String),
    /// Run an adapter's command: adapter name, command id.
    Adapter(String, String),
    /// Run a WebAssembly plugin's command: plugin name, command id.
    Plugin(String, String),
    /// Run a command registered by init.lua.
    Lua(String),
    /// Open this document (File → Open Recent). Not bindable in keys.toml.
    OpenFile(std::path::PathBuf),
}

impl Command {
    pub fn label(&self) -> String {
        match self {
            Command::Builtin(b) => entry(*b).2.to_owned(),
            Command::Style(s) => s.clone(),
            Command::Adapter(_, id) | Command::Plugin(_, id) => id.clone(),
            Command::Lua(name) => name.clone(),
            Command::OpenFile(p) => p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned()),
        }
    }
}

fn entry(b: Builtin) -> &'static (Builtin, &'static str, &'static str, &'static [&'static str]) {
    BUILTINS.iter().find(|e| e.0 == b).expect("every builtin is listed")
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::Builtin(b) => f.write_str(entry(*b).1),
            Command::Style(s) => write!(f, "style {s}"),
            Command::Adapter(name, id) => write!(f, "adapter \"{name}\" {id}"),
            Command::Plugin(name, id) => write!(f, "plugin \"{name}\" {id}"),
            Command::Lua(name) => write!(f, "lua {name}"),
            Command::OpenFile(p) => write!(f, "open {}", p.display()),
        }
    }
}

/// Parses a command name. `Ok(None)` means "none" (unbind).
pub fn parse_command(s: &str) -> Result<Option<Command>, String> {
    let s = s.trim();
    if s == "none" {
        return Ok(None);
    }
    if let Some(style) = s.strip_prefix("style ") {
        return Ok(Some(Command::Style(style.trim().to_owned())));
    }
    if let Some(name) = s.strip_prefix("lua ") {
        return Ok(Some(Command::Lua(name.trim().to_owned())));
    }
    // "component" is the older name for adapters and still works.
    for (prefix, kind) in [("adapter ", "adapter"), ("component ", "adapter"), ("plugin ", "plugin")] {
        let Some(rest) = s.strip_prefix(prefix) else { continue };
        let rest = rest.trim();
        let usage = format!("expected: {kind} \"Name\" command-id");
        let (name, id) = if let Some(quoted) = rest.strip_prefix('"') {
            let end = quoted.find('"').ok_or(format!("{kind} name is missing its closing quote"))?;
            (&quoted[..end], quoted[end + 1..].trim())
        } else {
            rest.split_once(' ').ok_or(usage.clone())?
        };
        if id.is_empty() {
            return Err(usage);
        }
        let (name, id) = (name.to_owned(), id.trim().to_owned());
        return Ok(Some(if kind == "plugin" {
            Command::Plugin(name, id)
        } else {
            Command::Adapter(name, id)
        }));
    }
    BUILTINS
        .iter()
        .find(|e| e.1 == s)
        .map(|e| Some(Command::Builtin(e.0)))
        .ok_or_else(|| format!("unknown command \"{s}\""))
}

/// A key combination such as cmd+shift+z.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub cmd: bool,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub key: Key,
}

impl Chord {
    pub fn parse(s: &str) -> Result<Self, String> {
        let lower = s.trim().to_lowercase();
        // "cmd++" binds the plus key.
        let (mods, key) = match lower.strip_suffix("++") {
            Some(m) => (m, "+"),
            None => lower.rsplit_once('+').unwrap_or(("", lower.as_str())),
        };
        let mut chord = Chord {
            cmd: false,
            shift: false,
            alt: false,
            ctrl: false,
            key: parse_key(key).ok_or_else(|| format!("unknown key \"{key}\" in \"{s}\""))?,
        };
        for m in mods.split('+').filter(|m| !m.is_empty()) {
            match m {
                "cmd" | "command" | "super" => chord.cmd = true,
                "shift" => chord.shift = true,
                "alt" | "opt" | "option" => chord.alt = true,
                "ctrl" | "control" => chord.ctrl = true,
                other => return Err(format!("unknown modifier \"{other}\" in \"{s}\"")),
            }
        }
        Ok(chord)
    }

    fn modifier_count(&self) -> usize {
        [self.cmd, self.shift, self.alt, self.ctrl].iter().filter(|b| **b).count()
    }

    /// Exact match, except that keys typed with Shift (like +) ignore Shift.
    pub fn matches(&self, key: Key, m: egui::Modifiers) -> bool {
        let shifted_symbol = matches!(
            key,
            Key::Plus | Key::Colon | Key::Questionmark | Key::Exclamationmark | Key::Pipe
                | Key::OpenCurlyBracket | Key::CloseCurlyBracket
        );
        // On Windows and Linux, "cmd" is the Ctrl key.
        let (cmd, ctrl) = if cfg!(target_os = "macos") {
            (self.cmd, self.ctrl)
        } else {
            (false, self.cmd || self.ctrl)
        };
        key == self.key
            && m.mac_cmd == cmd
            && m.alt == self.alt
            && m.ctrl == ctrl
            && (shifted_symbol || m.shift == self.shift)
    }

    /// The chord as menus show it: "Cmd+Shift+Z" on macOS, "Ctrl+Shift+Z"
    /// elsewhere.
    pub fn accelerator(&self) -> String {
        let mac = cfg!(target_os = "macos");
        let (cmd, ctrl) = if mac { (self.cmd, self.ctrl) } else { (false, self.cmd || self.ctrl) };
        let mut out = String::new();
        for (on, name) in [(cmd, "Cmd+"), (ctrl, "Ctrl+"), (self.alt, "Alt+"), (self.shift, "Shift+")] {
            if on {
                out += name;
            }
        }
        out + key_text(self.key)
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [(self.cmd, "cmd+"), (self.ctrl, "ctrl+"), (self.alt, "alt+"), (self.shift, "shift+")] {
            if on {
                f.write_str(name)?;
            }
        }
        f.write_str(&key_text(self.key).to_lowercase())
    }
}

/// How a key is written in keys.toml and the menu bar. egui's own symbols use
/// characters like "−" (U+2212) that the menu bar doesn't understand.
fn key_text(key: Key) -> &'static str {
    match key {
        Key::Minus => "-",
        Key::Equals => "=",
        Key::Plus => "+",
        Key::Period => ".",
        Key::Comma => ",",
        Key::Slash => "/",
        Key::Backslash => "\\",
        Key::Semicolon => ";",
        Key::Quote => "'",
        Key::Backtick => "`",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        Key::ArrowLeft => "Left",
        Key::ArrowRight => "Right",
        other => other.name(),
    }
}

fn parse_key(s: &str) -> Option<Key> {
    Key::from_name(s).or_else(|| {
        // egui spells named keys with a capital ("Escape", "F1").
        let mut c = s.chars();
        let capitalized: String = c.next()?.to_uppercase().chain(c).collect();
        Key::from_name(&capitalized)
    })
}

struct Binding {
    chord: Chord,
    command: Command,
    from_user: bool,
}

#[derive(Default)]
pub struct Keymap {
    bindings: Vec<Binding>,
}

impl Keymap {
    /// Builds the keymap: built-in defaults, then theme style keys and
    /// adapter shortcuts, then the user's `keys.toml` on top.
    /// Returns problems found in the user's file alongside the keymap.
    pub fn build(
        user: &BTreeMap<String, String>,
        defaults: impl IntoIterator<Item = (String, Command)>,
    ) -> (Self, Vec<String>) {
        let mut map = Keymap::default();
        let mut problems = Vec::new();
        let builtin_defaults = BUILTINS
            .iter()
            .flat_map(|(b, _, _, keys)| keys.iter().map(|k| (k.to_string(), Command::Builtin(*b))));
        for (keys, command) in builtin_defaults.chain(defaults) {
            match Chord::parse(&keys) {
                Ok(chord) => map.bind(chord, Some(command), false),
                Err(e) => problems.push(e),
            }
        }
        for (keys, name) in user {
            match (Chord::parse(keys), parse_command(name)) {
                (Ok(chord), Ok(command)) => map.bind(chord, command, true),
                (Err(e), _) | (_, Err(e)) => problems.push(format!("keys.toml: {e}")),
            }
        }
        // Most specific first, so cmd+shift+z is checked before cmd+z.
        map.bindings.sort_by_key(|b| std::cmp::Reverse(b.chord.modifier_count()));
        (map, problems)
    }

    fn bind(&mut self, chord: Chord, command: Option<Command>, from_user: bool) {
        self.bindings.retain(|b| b.chord != chord);
        if let Some(command) = command {
            self.bindings.push(Binding {
                chord,
                command,
                from_user,
            });
        }
    }

    /// The shortcut to show next to a command in the menu, preferring the user's.
    pub fn chord_for(&self, command: &Command) -> Option<Chord> {
        let mut matching = self.bindings.iter().filter(|b| &b.command == command);
        let first = matching.next()?;
        Some(
            std::iter::once(first)
                .chain(matching)
                .find(|b| b.from_user)
                .unwrap_or(first)
                .chord,
        )
    }

    /// Takes key presses that match a binding out of this frame's input.
    /// Bindings whose command `allow` rejects are left for other widgets.
    pub fn take_commands(&self, ctx: &egui::Context, allow: impl Fn(&Command) -> bool) -> Vec<Command> {
        let mut out = Vec::new();
        ctx.input_mut(|i| {
            i.events.retain(|e| {
                let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = e
                else {
                    return true;
                };
                match self.bindings.iter().find(|b| b.chord.matches(*key, *modifiers)) {
                    Some(b) if allow(&b.command) => {
                        out.push(b.command.clone());
                        false
                    }
                    _ => true,
                }
            });
        });
        out
    }

    /// Every binding, for the help overlay.
    pub fn describe(&self) -> Vec<(String, String)> {
        let mut rows: Vec<(String, String)> = self
            .bindings
            .iter()
            .map(|b| (b.chord.to_string(), b.command.to_string()))
            .collect();
        rows.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chords_and_commands() {
        let c = Chord::parse("Cmd+Shift+Z").unwrap();
        assert!(c.cmd && c.shift && !c.alt && c.key == Key::Z);
        assert_eq!(Chord::parse("cmd++").unwrap().key, Key::Plus);
        assert_eq!(Chord::parse("cmd+\\").unwrap().key, Key::Backslash);
        assert_eq!(Chord::parse("f5").unwrap().key, Key::F5);
        assert!(Chord::parse("hyper+x").is_err());

        assert_eq!(parse_command("bold").unwrap(), Some(Command::Builtin(Builtin::Bold)));
        assert_eq!(parse_command("style h2").unwrap(), Some(Command::Style("h2".into())));
        assert_eq!(
            parse_command("component \"Writing Tools\" goal").unwrap(),
            Some(Command::Adapter("Writing Tools".into(), "goal".into())),
            "the old name still works"
        );
        assert_eq!(parse_command("none").unwrap(), None);
        assert!(parse_command("frobnicate").is_err());
        assert_eq!(
            parse_command("plugin \"Word Goals\" set-goal").unwrap(),
            Some(Command::Plugin("Word Goals".into(), "set-goal".into()))
        );
        for cmd in ["save-as", "style quote", "adapter \"A B\" c", "plugin \"A B\" c"] {
            assert_eq!(parse_command(cmd).unwrap().unwrap().to_string(), cmd, "round trips");
        }
    }

    #[test]
    fn user_keys_override_and_unbind() {
        let user = BTreeMap::from([
            ("cmd+t".to_string(), "none".to_string()),
            ("cmd+shift+b".to_string(), "bold".to_string()),
            ("cmd+1".to_string(), "style quote".to_string()),
            ("cmd+k".to_string(), "nonsense".to_string()),
        ]);
        let defaults = [("cmd+1".to_string(), Command::Style("h1".into()))];
        let (map, problems) = Keymap::build(&user, defaults);

        assert_eq!(problems.len(), 1, "the bad command is reported: {problems:?}");
        assert_eq!(map.chord_for(&Command::Builtin(Builtin::NextTheme)), None);
        let bold = map.chord_for(&Command::Builtin(Builtin::Bold)).unwrap();
        assert!(bold.shift, "the user's binding is the one shown in the menu");
        assert_eq!(map.chord_for(&Command::Style("h1".into())), None, "replaced by the user");
        assert!(map.chord_for(&Command::Style("quote".into())).is_some());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn default_shortcuts_show_in_the_menu_bar() {
        for (_, name, _, keys) in BUILTINS {
            for k in keys.iter().filter(|k| !k.ends_with("++")) {
                let accel = Chord::parse(k).unwrap().accelerator();
                assert!(
                    accel.parse::<muda::accelerator::Accelerator>().is_ok(),
                    "{name}: menu can't show {accel}"
                );
            }
        }
    }

    #[test]
    fn matching_is_exact() {
        let undo = Chord::parse("cmd+z").unwrap();
        // What egui reports for the Cmd key on macOS, and Ctrl elsewhere.
        let cmd = if cfg!(target_os = "macos") {
            egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND
        } else {
            egui::Modifiers::CTRL | egui::Modifiers::COMMAND
        };
        let cmd_shift = cmd | egui::Modifiers::SHIFT;
        assert!(undo.matches(Key::Z, cmd));
        assert!(!undo.matches(Key::Z, cmd_shift));
        assert!(Chord::parse("cmd++").unwrap().matches(Key::Plus, cmd_shift));
    }
}
