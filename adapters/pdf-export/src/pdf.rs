//! Writes Folio's print model out with `krilla`. Folio lays the document out
//! exactly as on screen and hands over a [`Model`]: pages of positioned words
//! and rectangles, in PDF points. The theme's font files are embedded so the
//! PDF looks the same everywhere.

use std::collections::HashMap;
use std::path::PathBuf;

use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Rect, Size, Transform};
use krilla::metadata::Metadata;
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::Fill;
use krilla::text::{Font, TextDirection};
use krilla::{Data, Document};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

/// A document ready to print, in PDF points (1/72 inch) from each page's top left.
#[derive(Deserialize)]
pub struct Model {
    #[serde(default)]
    pub title: String,
    pub width: f32,
    pub height: f32,
    /// Page color, if the theme asks for it to be printed.
    #[serde(default, deserialize_with = "opt_color")]
    pub background: Option<Color>,
    pub pages: Vec<Page>,
}

#[derive(Default, Deserialize)]
pub struct Page {
    #[serde(default)]
    pub texts: Vec<Text>,
    #[serde(default)]
    pub rects: Vec<Block>,
}

/// One run of text (usually a word) at its baseline position.
#[derive(Deserialize)]
pub struct Text {
    pub x: f32,
    pub y: f32,
    pub text: String,
    /// Absolute path to the TTF/OTF file.
    pub font: PathBuf,
    pub size: f32,
    #[serde(deserialize_with = "color")]
    pub color: Color,
    /// Slant a regular face because the theme has no italic file.
    #[serde(default)]
    pub slant: bool,
}

/// A filled rectangle, e.g. an underline.
#[derive(Deserialize)]
pub struct Block {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(deserialize_with = "color")]
    pub color: Color,
}

/// sRGB with straight (unmultiplied) alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub [u8; 4]);

impl Color {
    /// Parses `#rrggbbaa` or `#rrggbb`.
    pub fn parse(s: &str) -> Option<Color> {
        let hex = s.strip_prefix('#')?;
        if !hex.is_ascii() || !matches!(hex.len(), 6 | 8) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        let a = if hex.len() == 8 { byte(6)? } else { 255 };
        Some(Color([byte(0)?, byte(2)?, byte(4)?, a]))
    }
}

fn color<'de, D: Deserializer<'de>>(d: D) -> Result<Color, D::Error> {
    let s = String::deserialize(d)?;
    Color::parse(&s).ok_or_else(|| D::Error::custom(format!("invalid color {s:?}")))
}

fn opt_color<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Color>, D::Error> {
    match Option::<String>::deserialize(d)? {
        None => Ok(None),
        Some(s) => Color::parse(&s).map(Some).ok_or_else(|| D::Error::custom(format!("invalid color {s:?}"))),
    }
}

pub fn render(model: &Model) -> Result<Vec<u8>, String> {
    let mut doc = Document::new();
    doc.set_metadata(Metadata::new().title(model.title.clone()).creator("Folio".to_owned()));
    let size = Size::from_wh(model.width, model.height).ok_or("the theme's page size is invalid")?;

    let mut fonts: HashMap<PathBuf, Font> = HashMap::new();
    let mut font_for = |path: &PathBuf| -> Result<Font, String> {
        if let Some(f) = fonts.get(path) {
            return Ok(f.clone());
        }
        let data: Data = std::fs::read(path)
            .map_err(|e| format!("couldn't read font {}: {e}", path.display()))?
            .into();
        let font = Font::new(data, 0).ok_or_else(|| format!("{} couldn't be read as a font", path.display()))?;
        fonts.insert(path.clone(), font.clone());
        Ok(font)
    };

    for page in &model.pages {
        let mut p = doc.start_page_with(PageSettings::new(size));
        let mut surface = p.surface();

        if let Some(bg) = model.background {
            fill_rect(&mut surface, 0.0, 0.0, model.width, model.height, bg);
        }
        for b in &page.rects {
            fill_rect(&mut surface, b.x, b.y, b.w, b.h, b.color);
        }
        for t in &page.texts {
            surface.set_fill(Some(fill(t.color)));
            if t.slant {
                // Lean glyphs right around their baseline: x' = x + 0.2 * (baseline - y).
                surface.push_transform(&Transform::from_row(1.0, 0.0, -0.2, 1.0, 0.2 * t.y, 0.0));
            }
            surface.draw_text(
                Point::from_xy(t.x, t.y),
                font_for(&t.font)?,
                t.size,
                &t.text,
                false,
                TextDirection::Auto,
            );
            if t.slant {
                surface.pop();
            }
        }
        surface.finish();
        p.finish();
    }
    doc.finish().map_err(|e| format!("couldn't write the PDF: {e:?}"))
}

fn fill(c: Color) -> Fill {
    let [r, g, b, a] = c.0;
    Fill {
        paint: rgb::Color::new(r, g, b).into(),
        opacity: NormalizedF32::new(f32::from(a) / 255.0).unwrap_or(NormalizedF32::ONE),
        ..Default::default()
    }
}

fn fill_rect(surface: &mut krilla::surface::Surface<'_>, x: f32, y: f32, w: f32, h: f32, c: Color) {
    let Some(rect) = Rect::from_xywh(x, y, w.max(0.01), h.max(0.01)) else { return };
    let mut pb = PathBuilder::new();
    pb.push_rect(rect);
    if let Some(path) = pb.finish() {
        surface.set_fill(Some(fill(c)));
        surface.draw_path(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: Color = Color([0, 0, 0, 255]);

    #[test]
    fn writes_a_pdf_with_text_and_pages() {
        let page = |word: &str| Page {
            texts: vec![Text {
                x: 72.0,
                y: 100.0,
                text: word.to_owned(),
                font: "/System/Library/Fonts/Supplemental/Georgia.ttf".into(),
                size: 12.0,
                color: BLACK,
                slant: true,
            }],
            rects: vec![Block { x: 72.0, y: 102.0, w: 40.0, h: 1.0, color: BLACK }],
        };
        let model = Model {
            title: "Test".to_owned(),
            width: 612.0,
            height: 792.0,
            background: None,
            pages: vec![page("Hello"), page("World"), Page::default()],
        };
        let bytes = render(&model).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(bytes.starts_with(b"%PDF-"));
        assert!(text.contains("/Type/Pages/Count 3"), "three pages");
        assert!(text.contains("FontFile2"), "the theme font is embedded");
        assert!(text.contains("/ToUnicode"), "text stays selectable and searchable");
        assert!(text.contains("/Title(Test)"));
    }

    #[test]
    fn reads_the_json_model() {
        let model: Model = serde_json::from_str(
            r##"{"title":"essay","width":612.0,"height":792.0,"background":"#fffaf0ff",
                "pages":[{"texts":[{"x":72.0,"y":100.0,"text":"Hi","font":"/f.ttf","size":12.0,"color":"#222222ff","slant":false}],
                          "rects":[{"x":72.0,"y":102.0,"w":40.0,"h":1.0,"color":"#22222280"}]},{}]}"##,
        )
        .unwrap();
        assert_eq!(model.title, "essay");
        assert_eq!(model.background, Some(Color([0xff, 0xfa, 0xf0, 0xff])));
        assert_eq!(model.pages.len(), 2);
        assert_eq!(model.pages[0].rects[0].color, Color([0x22, 0x22, 0x22, 0x80]));
        assert_eq!(Color::parse("#123456"), Some(Color([0x12, 0x34, 0x56, 0xff])));
        assert_eq!(Color::parse("123456"), None);
        assert!(serde_json::from_str::<Model>(r#"{"width":1,"height":1,"background":"red","pages":[]}"#).is_err());
    }
}
