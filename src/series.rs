//! text9000's place in the 9000 series: its own identity, and the four
//! sister apps it hands work to. Opening a video, sound, photo or comic (or
//! a .splice, .wave or .grace project), dropping one on the window, or
//! clicking a link to one in a document opens it in splice9000, wave9000,
//! grace9000 or leaf9000, whichever is installed. text9000 itself never
//! edits them.

use std::path::{Path, PathBuf};

/// What the app is called everywhere a person reads it.
pub const NAME: &str = "text9000";
/// The extension new documents get.
pub const EXTENSION: &str = "t9k";
/// Folio's extension. Folio documents open, and save back as .folio.
pub const FOLIO_EXTENSION: &str = "folio";

/// Whether text9000 opens this file as one of its own documents.
pub fn is_document(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION) || e.eq_ignore_ascii_case(FOLIO_EXTENSION))
}

/// An environment variable by its text9000 name, or by Folio's
/// (`TEXT9000_CONFIG_DIR`, then `FOLIO_CONFIG_DIR`).
pub fn env(suffix: &str) -> Option<std::ffi::OsString> {
    [format!("TEXT9000_{suffix}"), format!("FOLIO_{suffix}")]
        .into_iter()
        .find_map(|name| std::env::var_os(name).filter(|v| !v.is_empty()))
}

/// The series' window colors: quiet graphite with one signal color, here
/// text9000's ink green (splice orange, wave blue, grace violet). The
/// document keeps its own theme; this is everything around it.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub accent: egui::Color32,
    pub on_accent: egui::Color32,
    pub panel: egui::Color32,
    pub header: egui::Color32,
    pub line: egui::Color32,
    pub well: egui::Color32,
    pub text: egui::Color32,
    pub weak: egui::Color32,
}

pub const INK_GREEN: egui::Color32 = egui::Color32::from_rgb(0x3f, 0xd6, 0x8f);

pub const PALETTE: Palette = Palette {
    accent: INK_GREEN,
    on_accent: egui::Color32::from_rgb(0x08, 0x14, 0x0e),
    panel: egui::Color32::from_rgb(0x17, 0x18, 0x1b),
    header: egui::Color32::from_rgb(0x12, 0x13, 0x15),
    line: egui::Color32::from_rgb(0x2a, 0x2c, 0x31),
    well: egui::Color32::from_rgb(0x0c, 0x0d, 0x0f),
    text: egui::Color32::from_rgb(0xe8, 0xe9, 0xeb),
    weak: egui::Color32::from_rgb(0x84, 0x88, 0x90),
};

impl Palette {
    /// egui's own widgets (menus, the Log, Versions, prompts) in the same
    /// colors, as in the other 9000 apps.
    pub fn visuals(&self) -> egui::Visuals {
        use egui::Color32;
        let mut v = egui::Visuals::dark();
        v.panel_fill = self.panel;
        v.window_fill = self.panel;
        v.extreme_bg_color = self.well;
        v.faint_bg_color = self.header;
        v.window_stroke.color = self.line;
        v.selection.bg_fill = self.accent.gamma_multiply(0.35);
        v.selection.stroke.color = self.accent;
        v.hyperlink_color = self.accent;
        let w = &mut v.widgets;
        let (quiet, hover) = (Color32::from_rgb(0x22, 0x24, 0x28), Color32::from_rgb(0x2c, 0x2f, 0x34));
        for (state, fill) in [(&mut w.inactive, quiet), (&mut w.hovered, hover), (&mut w.active, hover), (&mut w.open, hover)] {
            state.weak_bg_fill = fill;
            state.bg_fill = fill;
            state.bg_stroke.color = Color32::TRANSPARENT;
            state.corner_radius = egui::CornerRadius::same(4);
        }
        w.noninteractive.bg_stroke.color = self.line;
        w.noninteractive.fg_stroke.color = self.text;
        w.inactive.fg_stroke.color = self.text;
        v
    }
}

/// text9000's mark, `size` points square, its left edge at `left`: three
/// lines of text, the last one cut on splice9000's slant into ink green.
pub fn mark(painter: &egui::Painter, left: egui::Pos2, size: f32, pal: &Palette) {
    use egui::{Shape, Stroke, pos2};
    let (x0, x1) = (left.x, left.x + size);
    let h = size * 0.17;
    let row = |i: f32| left.y - size * 0.36 + i * size * 0.36 - h / 2.0;
    let bar = |x0: f32, x1: f32, y: f32| egui::Rect::from_min_max(pos2(x0, y), pos2(x1, y + h));
    painter.rect_filled(bar(x0, x1, row(0.0)), h / 2.0, pal.text);
    painter.rect_filled(bar(x0, x1 - size * 0.18, row(1.0)), h / 2.0, pal.text);
    // The last line: light, then green, split by a slanted cut.
    let (y, cut, slant, gap) = (row(2.0), x0 + size * 0.42, h * 0.45, size * 0.06);
    let light = vec![pos2(x0, y), pos2(cut - slant, y), pos2(cut + slant, y + h), pos2(x0, y + h)];
    let green = vec![pos2(cut - slant + gap, y), pos2(x1 - size * 0.08, y), pos2(x1 - size * 0.08, y + h), pos2(cut + slant + gap, y + h)];
    painter.add(Shape::convex_polygon(light, pal.text, Stroke::NONE));
    painter.add(Shape::convex_polygon(green, pal.accent, Stroke::NONE));
}

/// The mark and "text9000" (the name, then 9000 in mono and green), as the
/// other 9000 apps draw theirs. Returns where it ends.
pub fn wordmark(painter: &egui::Painter, left: egui::Pos2, pal: &Palette) -> f32 {
    use egui::{Align2, FontId, pos2};
    mark(painter, pos2(left.x, left.y), 16.0, pal);
    let name = painter.text(pos2(left.x + 24.0, left.y), Align2::LEFT_CENTER, "text", FontId::proportional(14.0), pal.text);
    let nums = painter.text(pos2(name.right() + 1.0, left.y + 0.5), Align2::LEFT_CENTER, "9000", FontId::monospace(12.5), pal.accent);
    nums.right()
}

pub struct Sibling {
    pub name: &'static str,
    /// What it does, for messages ("the video editor").
    pub kind: &'static str,
    /// How macOS finds it.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub bundle_id: &'static str,
    /// Its own project files, then the media it opens.
    pub extensions: &'static [&'static str],
    pub repo: &'static str,
}

pub const SIBLINGS: [Sibling; 4] = [
    Sibling {
        name: "splice9000",
        kind: "video editor",
        bundle_id: "app.splice9000.editor",
        extensions: &[
            "splice", "mp4", "mov", "m4v", "mkv", "webm", "avi", "mpg", "mpeg", "mts", "m2ts", "wmv", "flv", "3gp",
        ],
        repo: "https://github.com/solacecasino/splice9000",
    },
    Sibling {
        name: "wave9000",
        kind: "audio workstation",
        bundle_id: "app.wave9000.workstation",
        extensions: &[
            "wave", "wav", "mp3", "flac", "aiff", "aif", "m4a", "aac", "ogg", "oga", "opus", "wma", "caf",
        ],
        repo: "https://github.com/solacecasino/wave9000",
    },
    Sibling {
        name: "grace9000",
        kind: "photo editor",
        bundle_id: "app.grace9000.editor",
        extensions: &[
            "grace", "jpg", "jpeg", "png", "tif", "tiff", "webp", "heic", "heif", "avif", "bmp", "gif", "jxl", "dng",
            "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "raf", "orf", "rw2", "pef", "srw", "3fr", "iiq", "x3f",
        ],
        repo: "https://github.com/solacecasino/grace9000",
    },
    Sibling {
        name: "leaf9000",
        kind: "comic reader",
        bundle_id: "app.leaf9000.reader",
        extensions: &["cbz", "cbr", "cb7", "cbt"],
        repo: "https://github.com/solacecasino/leaf9000",
    },
];

pub fn sibling(name: &str) -> Option<&'static Sibling> {
    SIBLINGS.iter().find(|s| s.name == name)
}

/// The sister app that opens this file, by its extension.
pub fn for_file(path: &Path) -> Option<&'static Sibling> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    SIBLINGS.iter().find(|s| s.extensions.contains(&ext.as_str()))
}

/// Where a sister app is installed, if it is. `configured` is the path
/// from settings.toml's `[apps]`, which wins.
pub fn find(s: &Sibling, configured: Option<&str>) -> Option<PathBuf> {
    if let Some(path) = configured.map(PathBuf::from).filter(|p| p.exists()) {
        return Some(path);
    }
    os::find(s)
}

/// Opens a sister app, with a file if given. Returns why it couldn't.
pub fn open(s: &Sibling, file: Option<&Path>, configured: Option<&str>) -> Result<(), String> {
    let Some(app) = find(s, configured) else {
        return Err(format!(
            "{} isn't installed. It's the {} in the 9000 series: {}/releases/latest",
            s.name, s.kind, s.repo
        ));
    };
    os::launch(&app, file).map_err(|e| format!("Couldn't open {}: {e}", s.name))
}

#[cfg(target_os = "macos")]
mod os {
    use std::path::{Path, PathBuf};

    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSString;

    use super::Sibling;

    pub fn find(s: &Sibling) -> Option<PathBuf> {
        let url = NSWorkspace::sharedWorkspace().URLForApplicationWithBundleIdentifier(&NSString::from_str(s.bundle_id))?;
        url.path().map(|p| PathBuf::from(p.to_string()))
    }

    pub fn launch(app: &Path, file: Option<&Path>) -> std::io::Result<()> {
        let mut cmd = std::process::Command::new("/usr/bin/open");
        cmd.arg("-a").arg(app);
        if let Some(file) = file {
            cmd.arg(file);
        }
        let status = cmd.status()?;
        if status.success() { Ok(()) } else { Err(std::io::Error::other(format!("open exited with {status}"))) }
    }
}

#[cfg(not(target_os = "macos"))]
mod os {
    use std::path::{Path, PathBuf};

    use super::Sibling;

    fn exe_name(s: &Sibling) -> String {
        if cfg!(windows) { format!("{}.exe", s.name) } else { s.name.to_owned() }
    }

    pub fn find(s: &Sibling) -> Option<PathBuf> {
        let exe = exe_name(s);
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(path) = std::env::var_os("PATH") {
            candidates.extend(std::env::split_paths(&path).map(|dir| dir.join(&exe)));
        }
        // The downloads are folders you unpack anywhere: look beside
        // text9000's own folder (Downloads/splice9000-0.0.8-windows-x64/...).
        if let Some(parent) = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.parent()?.to_path_buf()))
            && let Ok(entries) = std::fs::read_dir(&parent)
        {
            let mut beside: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().to_ascii_lowercase().starts_with(s.name))
                .map(|e| e.path().join(&exe))
                .collect();
            // The newest version sorts last.
            beside.sort();
            candidates.extend(beside.into_iter().rev());
        }
        if cfg!(windows) {
            if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                candidates.push(PathBuf::from(local).join("Programs").join(s.name).join(&exe));
            }
        } else {
            candidates.extend(desktop_entry(s));
        }
        candidates.into_iter().find(|p| p.is_file())
    }

    /// The program a sister app's menu entry starts (its install.sh writes
    /// one with the full path).
    fn desktop_entry(s: &Sibling) -> Vec<PathBuf> {
        let home = std::env::home_dir().unwrap_or_default();
        let data = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map_or_else(|| home.join(".local/share"), PathBuf::from);
        [data, PathBuf::from("/usr/local/share"), PathBuf::from("/usr/share")]
            .iter()
            .filter_map(|dir| std::fs::read_to_string(dir.join("applications").join(format!("{}.desktop", s.name))).ok())
            .filter_map(|entry| {
                let exec = entry.lines().find_map(|l| l.strip_prefix("Exec="))?;
                Some(PathBuf::from(super::exec_program(exec)))
            })
            .collect()
    }

    pub fn launch(app: &Path, file: Option<&Path>) -> std::io::Result<()> {
        let mut cmd = std::process::Command::new(app);
        if let Some(file) = file {
            cmd.arg(file);
        }
        crate::platform::hide_console(&mut cmd);
        cmd.spawn().map(drop)
    }
}

/// The program in a desktop entry's Exec= line: the first word, or the
/// quoted path. Desktop files escape twice (the file's strings, then the
/// quoted argument), so `\\$` in the file is `$`.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn exec_program(exec: &str) -> String {
    let unescaped = exec.trim().replace("\\\\", "\\");
    let exec = unescaped.as_str();
    if let Some(quoted) = exec.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = quoted.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => out.extend(chars.next()),
                c => out.push(c),
            }
        }
        out
    } else {
        exec.split_whitespace().next().unwrap_or_default().to_owned()
    }
}

/// A link in a document that points at a file (`file://...`, an absolute
/// path, or a path relative to the document's folder), resolved.
pub fn link_target(url: &str, document: Option<&Path>) -> Option<PathBuf> {
    let lower = url.to_ascii_lowercase();
    if ["http://", "https://", "mailto:", "folio://", "text9000://"].iter().any(|s| lower.starts_with(s)) {
        return None;
    }
    let path = match url.strip_prefix("file://") {
        Some(rest) => {
            let rest = percent_decode(rest);
            // file:///C:/Users/... on Windows: the path starts at the drive.
            let drive = rest.len() > 2 && rest.as_bytes()[0] == b'/' && rest.as_bytes()[2] == b':';
            PathBuf::from(if cfg!(windows) && drive { &rest[1..] } else { &rest[..] })
        }
        None => PathBuf::from(url),
    };
    if path.is_absolute() {
        return Some(path);
    }
    Some(document?.parent()?.join(path))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// How a link to `file` is written in `document`: relative to its folder
/// when the file is in it or below, so a folder of notes and media can be
/// moved together.
pub fn link_for(file: &Path, document: Option<&Path>) -> String {
    let relative = document.and_then(|d| d.parent()).and_then(|dir| file.strip_prefix(dir).ok());
    match relative {
        Some(rel) => rel.to_string_lossy().replace('\\', "/"),
        None => file.to_string_lossy().into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_go_to_the_right_sister() {
        let name = |p: &str| for_file(Path::new(p)).map(|s| s.name);
        assert_eq!(name("/m/cut.splice"), Some("splice9000"));
        assert_eq!(name("/m/Clip.MOV"), Some("splice9000"));
        assert_eq!(name("take.wave"), Some("wave9000"));
        assert_eq!(name("song.flac"), Some("wave9000"));
        assert_eq!(name("IMG_0001.CR3"), Some("grace9000"));
        assert_eq!(name("look.grace"), Some("grace9000"));
        assert_eq!(name("Vol 01.CBZ"), Some("leaf9000"));
        assert_eq!(name("issue.cbr"), Some("leaf9000"));
        assert_eq!(name("notes.t9k"), None);
        assert_eq!(name("old.folio"), None);
        assert_eq!(name("README"), None);
        assert!(is_document(Path::new("a.T9K")) && is_document(Path::new("a.folio")));
    }

    #[test]
    fn links_to_files_resolve_beside_the_document() {
        let doc = Path::new("/notes/film/script.t9k");
        assert_eq!(link_target("shots/a.mov", Some(doc)), Some(PathBuf::from("/notes/film/shots/a.mov")));
        assert_eq!(link_target("file:///Users/me/My%20Take.wav", Some(doc)), Some(PathBuf::from("/Users/me/My Take.wav")));
        let absolute = if cfg!(windows) { r"C:\abs\cover.png" } else { "/abs/cover.png" };
        assert_eq!(link_target(absolute, None), Some(PathBuf::from(absolute)));
        if cfg!(windows) {
            assert_eq!(link_target("file:///C:/My%20Takes/a.wav", None), Some(PathBuf::from("C:/My Takes/a.wav")));
        }
        assert_eq!(link_target("https://example.com/a.mov", Some(doc)), None);
        assert_eq!(link_target("relative.mov", None), None, "an unsaved document has no folder");
        assert_eq!(link_for(Path::new("/notes/film/shots/a.mov"), Some(doc)), "shots/a.mov");
        assert_eq!(link_for(Path::new("/elsewhere/b.wav"), Some(doc)), "/elsewhere/b.wav");
    }

    #[test]
    fn desktop_entry_programs() {
        assert_eq!(exec_program(r#""/home/me/apps/splice 9000/splice9000""#), "/home/me/apps/splice 9000/splice9000");
        assert_eq!(exec_program(r#""/a/\\$x/wave9000" %f"#), "/a/$x/wave9000");
        assert_eq!(exec_program("grace9000 %F"), "grace9000");
    }
}
