//! PDF export. The editor lays the document out exactly as on screen and
//! hands over a [`Model`]: pages of positioned words and rectangles, in PDF
//! points. This module only writes that model out with `krilla`, embedding
//! the theme's font files so the PDF looks the same everywhere.

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui::{Color32, FontDefinitions};
use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Rect, Size, Transform};
use krilla::metadata::Metadata;
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::Fill;
use krilla::text::{Font, TextDirection};
use krilla::{Data, Document};

/// A document ready to print, in PDF points (1/72 inch) from each page's top left.
pub struct Model {
    pub width: f32,
    pub height: f32,
    /// Page color, if the theme asks for it to be printed.
    pub background: Option<Color32>,
    pub pages: Vec<Page>,
}

#[derive(Default)]
pub struct Page {
    pub texts: Vec<Text>,
    pub rects: Vec<Block>,
}

/// One run of text (usually a word) at its baseline position.
pub struct Text {
    pub x: f32,
    pub y: f32,
    pub text: String,
    /// The font file; None means egui's built-in font.
    pub font: Option<PathBuf>,
    pub size: f32,
    pub color: Color32,
    /// Slant a regular face because the theme has no italic file.
    pub slant: bool,
}

/// A filled rectangle, e.g. an underline.
pub struct Block {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: Color32,
}

pub fn render(model: &Model, title: &str) -> Result<Vec<u8>, String> {
    let mut doc = Document::new();
    doc.set_metadata(Metadata::new().title(title.to_owned()).creator("Folio".to_owned()));
    let size = Size::from_wh(model.width, model.height).ok_or("the theme's page size is invalid")?;

    let mut fonts: HashMap<Option<PathBuf>, Font> = HashMap::new();
    let mut font_for = |file: &Option<PathBuf>| -> Result<Font, String> {
        if let Some(f) = fonts.get(file) {
            return Ok(f.clone());
        }
        let data: Data = match file {
            Some(path) => std::fs::read(path)
                .map_err(|e| format!("couldn't read font {}: {e}", path.display()))?
                .into(),
            None => FontDefinitions::default()
                .font_data
                .get("Ubuntu-Light")
                .ok_or("built-in font missing")?
                .font
                .clone()
                .into(),
        };
        let font = Font::new(data, 0).ok_or("a theme font couldn't be read as a font")?;
        fonts.insert(file.clone(), font.clone());
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

fn fill(c: Color32) -> Fill {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Fill {
        paint: rgb::Color::new(r, g, b).into(),
        opacity: NormalizedF32::new(f32::from(a) / 255.0).unwrap_or(NormalizedF32::ONE),
        ..Default::default()
    }
}

fn fill_rect(surface: &mut krilla::surface::Surface<'_>, x: f32, y: f32, w: f32, h: f32, c: Color32) {
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

    #[test]
    fn writes_a_pdf_with_text_and_pages() {
        let page = |word: &str| Page {
            texts: vec![Text {
                x: 72.0,
                y: 100.0,
                text: word.to_owned(),
                font: Some("/System/Library/Fonts/Supplemental/Georgia.ttf".into()),
                size: 12.0,
                color: Color32::BLACK,
                slant: true,
            }],
            rects: vec![Block { x: 72.0, y: 102.0, w: 40.0, h: 1.0, color: Color32::BLACK }],
        };
        let model = Model {
            width: 612.0,
            height: 792.0,
            background: None,
            pages: vec![page("Hello"), page("World"), Page::default()],
        };
        let bytes = render(&model, "Test").unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(bytes.starts_with(b"%PDF-"));
        assert!(text.contains("/Type/Pages/Count 3"), "three pages");
        assert!(text.contains("FontFile2"), "the theme font is embedded");
        assert!(text.contains("/ToUnicode"), "text stays selectable and searchable");
        assert!(text.contains("/Title(Test)"));
    }
}
