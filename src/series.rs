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

/// Who text9000 is, for core9000: the 9000 series' shared window, adapter
/// host, family protocol and sisters.
pub static APP: core9000::App = core9000::App {
    name: NAME,
    version: env!("CARGO_PKG_VERSION"),
    env_prefix: "TEXT9000",
    legacy_env_prefix: "FOLIO",
    protocol: crate::adapters::PROTOCOL_VERSION,
    permissions: &[(crate::adapters::READ, "read the open document"), (crate::adapters::EDIT, "change the open document")],
    events_permission: crate::adapters::READ,
};

/// A sister app (from core9000, shared by the whole series).
pub type Sibling = core9000::sisters::Sister;

/// The four sisters text9000 hands work to.
pub fn siblings() -> impl Iterator<Item = &'static Sibling> {
    core9000::sisters::others()
}

pub fn sibling(name: &str) -> Option<&'static Sibling> {
    siblings().find(|s| s.name.eq_ignore_ascii_case(name))
}

/// The sister app that opens this file, by its extension.
pub fn for_file(path: &Path) -> Option<&'static Sibling> {
    core9000::sisters::for_file(path)
}

/// Where a sister app is installed, if it is. `configured` is the path
/// from settings.toml's `[apps]`, which wins.
pub fn find(s: &Sibling, configured: Option<&str>) -> Option<PathBuf> {
    s.find(configured.map(Path::new))
}

/// Opens a sister app, with a file if given. Returns why it couldn't.
pub fn open(s: &Sibling, file: Option<&Path>, configured: Option<&str>) -> Result<(), String> {
    s.open(file.as_slice(), configured.map(Path::new))
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
        core9000::init(&APP);
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
}
