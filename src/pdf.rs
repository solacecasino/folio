//! The print model: the document laid out exactly as on screen, as pages of
//! positioned words in PDF points. Folio itself doesn't write PDFs; it hands
//! this model to adapters (`doc.print_model`), and the PDF Export adapter
//! turns it into a file.

use std::path::PathBuf;

use egui::{Color32, FontDefinitions};
use serde::Serialize;
use serde_json::Value;

/// A document ready to print, in PDF points (1/72 inch) from each page's top left.
#[derive(Serialize)]
pub struct Model {
    pub width: f32,
    pub height: f32,
    /// Page color, if the theme asks for it to be printed.
    #[serde(serialize_with = "hex_opt")]
    pub background: Option<Color32>,
    pub pages: Vec<Page>,
}

#[derive(Default, Serialize)]
pub struct Page {
    pub texts: Vec<Text>,
    pub rects: Vec<Block>,
    /// Areas that are links, for clickable links in PDFs.
    pub links: Vec<LinkArea>,
}

/// A clickable area (one line of a link's text) and where it goes.
#[derive(Serialize)]
pub struct LinkArea {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub url: String,
}

/// One run of text (usually a word) at its baseline position.
#[derive(Serialize)]
pub struct Text {
    pub x: f32,
    pub y: f32,
    pub text: String,
    /// The font file; None means egui's built-in font (sent as a real path).
    pub font: Option<PathBuf>,
    pub size: f32,
    #[serde(serialize_with = "hex")]
    pub color: Color32,
    /// Slant a regular face because the theme has no italic file.
    pub slant: bool,
}

/// A filled rectangle, e.g. an underline.
#[derive(Serialize)]
pub struct Block {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(serialize_with = "hex")]
    pub color: Color32,
}

fn hex<S: serde::Serializer>(c: &Color32, s: S) -> Result<S::Ok, S::Error> {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    s.serialize_str(&format!("#{r:02x}{g:02x}{b:02x}{a:02x}"))
}

fn hex_opt<S: serde::Serializer>(c: &Option<Color32>, s: S) -> Result<S::Ok, S::Error> {
    match c {
        Some(c) => hex(c, s),
        None => s.serialize_none(),
    }
}

/// The model as adapters receive it: every font is a file path they can
/// read, including Folio's built-in font, which is written out once.
pub fn to_json(model: &Model, title: &str) -> Result<Value, String> {
    let builtin = builtin_font_path()?;
    let mut value = serde_json::to_value(model).map_err(|e| e.to_string())?;
    value["title"] = Value::String(title.to_owned());
    for page in value["pages"].as_array_mut().into_iter().flatten() {
        for text in page["texts"].as_array_mut().into_iter().flatten() {
            if text["font"].is_null() {
                text["font"] = Value::String(builtin.display().to_string());
            }
        }
    }
    Ok(value)
}

fn builtin_font_path() -> Result<PathBuf, String> {
    builtin_font("Ubuntu-Light")
}

/// One of egui's built-in fonts as a file, written to the cache once.
fn builtin_font(name: &str) -> Result<PathBuf, String> {
    let path = crate::settings::config_dir().join(format!("cache/{name}.ttf"));
    if !path.exists() {
        let defs = FontDefinitions::default();
        let font = defs.font_data.get(name).ok_or("built-in font missing")?;
        std::fs::create_dir_all(path.parent().expect("has parent")).map_err(|e| e.to_string())?;
        std::fs::write(&path, (*font.font).as_ref()).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

/// Whether a font file has a glyph for `c`. Results are cached; a file that
/// can't be read or parsed has none.
fn covers(path: &std::path::Path, c: char) -> bool {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static FILES: Mutex<Option<HashMap<PathBuf, Option<Vec<u8>>>>> = Mutex::new(None);
    let mut files = FILES.lock().unwrap_or_else(|e| e.into_inner());
    let data = files
        .get_or_insert_with(HashMap::new)
        .entry(path.to_path_buf())
        .or_insert_with(|| std::fs::read(path).ok());
    data.as_deref()
        .and_then(|d| ttf_parser::Face::parse(d, 0).ok())
        .is_some_and(|face| face.glyph_index(c).is_some())
}

/// The font to print `c` with: the theme's (`None` is the built-in font)
/// if it has the character, otherwise the first fallback font that does,
/// the same ones used on screen.
pub fn font_for(preferred: Option<&std::path::Path>, c: char) -> Option<PathBuf> {
    let has = match preferred {
        Some(p) => covers(p, c),
        None => builtin_font_path().is_ok_and(|p| covers(&p, c)),
    };
    if has {
        return preferred.map(PathBuf::from);
    }
    // The same order as on screen: egui's built-in fonts, then the system's
    // symbol font.
    static CHAIN: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
    let chain = CHAIN.get_or_init(|| {
        let defs = FontDefinitions::default();
        let names = defs.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
        let mut chain: Vec<PathBuf> = names.iter().filter_map(|name| builtin_font(name).ok()).collect();
        chain.extend(crate::theme::symbol_fonts());
        chain
    });
    chain.iter().find(|p| covers(p, c)).cloned().or_else(|| preferred.map(PathBuf::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_has_hex_colors_and_real_font_paths() {
        let model = Model {
            width: 612.0,
            height: 792.0,
            background: None,
            pages: vec![Page {
                texts: vec![Text {
                    x: 72.0,
                    y: 100.0,
                    text: "Hi".into(),
                    font: None,
                    size: 12.0,
                    color: Color32::from_rgb(0x22, 0x22, 0x22),
                    slant: false,
                }],
                rects: vec![Block { x: 1.0, y: 2.0, w: 3.0, h: 4.0, color: Color32::BLACK }],
                links: Vec::new(),
            }],
        };
        let v = to_json(&model, "essay").unwrap();
        assert_eq!(v["title"], "essay");
        assert_eq!(v["background"], Value::Null);
        assert_eq!(v["pages"][0]["texts"][0]["color"], "#222222ff");
        let font = v["pages"][0]["texts"][0]["font"].as_str().unwrap();
        assert!(std::path::Path::new(font).exists(), "the built-in font is on disk for adapters");
        assert_eq!(v["pages"][0]["rects"][0]["w"], 3.0);
    }

    #[test]
    fn missing_characters_fall_back_to_a_font_that_has_them() {
        // Georgia, or whatever serif the system has, has letters but no arrows.
        let theme: crate::theme::Theme = toml::from_str(crate::theme::DEFAULT_THEME).unwrap();
        let ctx = egui::Context::default();
        let fonts = crate::theme::install_fonts(&ctx, std::slice::from_ref(&theme));
        let (serif, _) = fonts.file(&format!("{}:serif", theme.name), false, false);
        let Some(serif) = serif else { return }; // no system serif on this machine
        assert_eq!(font_for(Some(&serif), 'a').as_deref(), Some(serif.as_path()), "letters stay in the theme font");
        let arrow = font_for(Some(&serif), '→').unwrap();
        if covers(&serif, '→') {
            assert_eq!(arrow, serif, "a font that has the character keeps it (DejaVu Serif does)");
        } else if !crate::theme::symbol_fonts().is_empty() {
            assert_ne!(arrow, serif, "Georgia has no arrows");
            assert!(covers(&arrow, '→'), "the fallback really has the arrow");
        }
    }
}
