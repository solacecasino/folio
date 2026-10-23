//! Themes are plain TOML files. Each one sets the page, colors, fonts and
//! paragraph styles; nothing about a document's look is hardcoded.
//!
//! Folio looks for themes inside Folio.app, in `./themes`, and in
//! `~/Library/Application Support/Folio/themes` (the user's own folder).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct Theme {
    pub name: String,
    #[serde(default)]
    pub page: PageSetup,
    #[serde(default)]
    pub colors: Colors,
    /// Font families by name, e.g. `[fonts.serif]`.
    #[serde(default)]
    pub fonts: BTreeMap<String, FontFiles>,
    /// Paragraph styles by name. `body` should always exist.
    #[serde(default)]
    pub styles: BTreeMap<String, Style>,
    /// Folder the theme was loaded from; font paths are relative to it.
    #[serde(skip)]
    pub dir: PathBuf,
}

/// Page geometry in inches.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct PageSetup {
    pub width: f32,
    pub height: f32,
    pub margin_top: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub margin_right: f32,
    /// Gap between pages on screen.
    pub gap: f32,
}

impl Default for PageSetup {
    fn default() -> Self {
        Self {
            width: 8.5,
            height: 11.0,
            margin_top: 1.0,
            margin_bottom: 1.0,
            margin_left: 1.0,
            margin_right: 1.0,
            gap: 0.3,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Colors {
    pub background: String,
    pub page: String,
    pub page_border: String,
    pub text: String,
    pub cursor: String,
    pub selection: String,
    pub status: String,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            background: "#3a3a3a".into(),
            page: "#ffffff".into(),
            page_border: "#00000040".into(),
            text: "#1e1e1e".into(),
            cursor: "#1e1e1e".into(),
            selection: "#3d7eff55".into(),
            status: "#b0b0b0".into(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct FontFiles {
    pub regular: Option<String>,
    pub bold: Option<String>,
    pub italic: Option<String>,
    pub bold_italic: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Style {
    /// Name of a family in `[fonts]`; unknown names use the built-in font.
    pub font: String,
    /// Size in typographic points.
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub color: Option<String>,
    /// Line height as a multiple of the font size.
    pub line_height: f32,
    /// Space before/after the paragraph, in points.
    pub space_before: f32,
    pub space_after: f32,
    /// First-line indent in inches.
    pub indent: f32,
    pub align: Align,
    /// Style for the paragraph created when pressing Enter at the end.
    pub next: Option<String>,
    /// Cmd+<digit> applies this style.
    pub key: Option<u8>,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            font: "serif".into(),
            size: 12.0,
            bold: false,
            italic: false,
            color: None,
            line_height: 1.3,
            space_before: 0.0,
            space_after: 6.0,
            indent: 0.0,
            align: Align::Left,
            next: None,
            key: None,
        }
    }
}

impl Theme {
    pub fn style(&self, name: &str) -> Style {
        self.styles
            .get(name)
            .or_else(|| self.styles.get("body"))
            .cloned()
            .unwrap_or_default()
    }

    pub fn next_style(&self, name: &str) -> String {
        self.styles
            .get(name)
            .and_then(|s| s.next.clone())
            .unwrap_or_else(|| name.to_owned())
    }

    pub fn style_for_key(&self, digit: u8) -> Option<&str> {
        self.styles
            .iter()
            .find(|(_, s)| s.key == Some(digit))
            .map(|(n, _)| n.as_str())
    }

    fn resolve_path(&self, p: &str) -> PathBuf {
        if let Some(rest) = p.strip_prefix("~/")
            && let Some(home) = std::env::var_os("HOME")
        {
            return PathBuf::from(home).join(rest);
        }
        let path = PathBuf::from(p);
        if path.is_absolute() {
            path
        } else {
            self.dir.join(path)
        }
    }
}

/// The built-in theme, used when no theme files are found.
pub const DEFAULT_THEME: &str = include_str!("../themes/paper.toml");

pub fn load_themes() -> Vec<Theme> {
    let mut dirs = Vec::new();
    // Inside Folio.app the bundled themes live in Contents/Resources.
    if let Ok(exe) = std::env::current_exe()
        && let Some(macos_dir) = exe.parent()
    {
        dirs.push(macos_dir.join("../Resources/themes"));
    }
    dirs.push(PathBuf::from("themes"));
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join("Library/Application Support/Folio/themes"));
    }

    let mut themes: Vec<Theme> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .collect();
        paths.sort();
        for path in paths {
            match load_theme(&path) {
                // A theme in the user's folder replaces a bundled one of the same name.
                Ok(t) => {
                    themes.retain(|existing| existing.name != t.name);
                    themes.push(t);
                }
                Err(e) => eprintln!("folio: skipping theme {}: {e}", path.display()),
            }
        }
    }

    if themes.is_empty() {
        let mut t: Theme = toml::from_str(DEFAULT_THEME).expect("built-in theme parses");
        t.dir = PathBuf::from(".");
        themes.push(t);
    }
    themes
}

fn load_theme(path: &Path) -> Result<Theme, String> {
    let src = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut theme: Theme = toml::from_str(&src).map_err(|e| e.to_string())?;
    theme.dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    Ok(theme)
}

/// Parses `#rgb`, `#rrggbb` or `#rrggbbaa`.
pub fn color(s: &str) -> Color32 {
    let hex = s.trim().trim_start_matches('#');
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(255);
    match hex.len() {
        3 => {
            let nib = |i: usize| {
                let v = u8::from_str_radix(&hex[i..=i], 16).unwrap_or(15);
                v * 17
            };
            Color32::from_rgb(nib(0), nib(1), nib(2))
        }
        8 => Color32::from_rgba_unmultiplied(byte(0), byte(2), byte(4), byte(6)),
        _ => Color32::from_rgb(byte(0), byte(2), byte(4)),
    }
}

/// Maps (theme family, bold, italic) to a registered egui family.
/// `synthetic_italic` is set when the theme has no italic file, so the
/// renderer slants the regular face instead.
#[derive(Default)]
pub struct FontMap {
    faces: HashMap<(String, bool, bool), FontFamily>,
}

pub struct Face {
    pub family: FontFamily,
    pub synthetic_italic: bool,
}

impl FontMap {
    pub fn face(&self, family: &str, bold: bool, italic: bool) -> Face {
        let get = |b, i| self.faces.get(&(family.to_owned(), b, i)).cloned();
        if let Some(f) = get(bold, italic) {
            return Face { family: f, synthetic_italic: false };
        }
        // Fall back to the nearest face that exists.
        let fallback = get(bold, false)
            .or_else(|| get(false, italic))
            .or_else(|| get(false, false))
            .unwrap_or(FontFamily::Proportional);
        Face {
            family: fallback,
            synthetic_italic: italic,
        }
    }
}

/// Registers every font from every theme with egui up front, so switching
/// themes never references a font that isn't loaded yet.
pub fn install_fonts(ctx: &egui::Context, themes: &[Theme]) -> FontMap {
    let mut defs = FontDefinitions::default();
    let builtin = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut map = FontMap::default();
    let mut loaded: HashMap<PathBuf, String> = HashMap::new();

    for theme in themes {
        for (family, files) in &theme.fonts {
            let variants = [
                (false, false, &files.regular),
                (true, false, &files.bold),
                (false, true, &files.italic),
                (true, true, &files.bold_italic),
            ];
            for (bold, italic, file) in variants {
                let Some(file) = file else { continue };
                let path = theme.resolve_path(file);
                let key = match loaded.get(&path) {
                    Some(k) => k.clone(),
                    None => {
                        let Ok(bytes) = std::fs::read(&path) else {
                            eprintln!("folio: font not found: {}", path.display());
                            continue;
                        };
                        let k = format!("file:{}", path.display());
                        defs.font_data.insert(k.clone(), FontData::from_owned(bytes).into());
                        loaded.insert(path, k.clone());
                        k
                    }
                };
                let egui_family = FontFamily::Name(format!("{}:{family}:{bold}:{italic}", theme.name).into());
                // Built-in fonts stay as fallbacks for glyphs the theme font lacks.
                let mut chain = vec![key];
                chain.extend(builtin.iter().cloned());
                defs.families.insert(egui_family.clone(), chain);
                map.faces
                    .insert((format!("{}:{family}", theme.name), bold, italic), egui_family);
            }
        }
    }

    ctx.set_fonts(defs);
    map
}
