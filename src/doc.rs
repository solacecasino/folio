//! The document model: a list of styled paragraphs, each holding characters
//! with per-character formatting. Kept independent of the UI so plugins and
//! exporters can work on it directly later.

use serde::{Deserialize, Serialize};

/// Inline formatting for a single character.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fmt {
    #[serde(default, skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub underline: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Clone, Debug)]
pub struct Paragraph {
    /// Name of a style defined by the active theme ("body", "h1", ...).
    pub style: String,
    pub chars: Vec<char>,
    /// Same length as `chars`.
    pub fmts: Vec<Fmt>,
}

impl Paragraph {
    pub fn new(style: &str) -> Self {
        Self {
            style: style.to_owned(),
            chars: Vec::new(),
            fmts: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    /// Runs of identically formatted text, in order.
    pub fn runs(&self) -> Vec<(String, Fmt)> {
        let mut runs: Vec<(String, Fmt)> = Vec::new();
        for (c, f) in self.chars.iter().zip(&self.fmts) {
            match runs.last_mut() {
                Some((text, fmt)) if fmt == f => text.push(*c),
                _ => runs.push((c.to_string(), *f)),
            }
        }
        runs
    }
}

/// A position between characters: paragraph index, then character index.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos {
    pub para: usize,
    pub ch: usize,
}

impl Pos {
    pub fn new(para: usize, ch: usize) -> Self {
        Self { para, ch }
    }
}

#[derive(Clone, Debug)]
pub struct Document {
    pub paragraphs: Vec<Paragraph>,
    /// Theme the document was last saved with, if any.
    pub theme: Option<String>,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            paragraphs: vec![Paragraph::new("body")],
            theme: None,
        }
    }
}

impl Document {
    pub fn end(&self) -> Pos {
        let last = self.paragraphs.len() - 1;
        Pos::new(last, self.paragraphs[last].len())
    }

    pub fn clamp(&self, p: Pos) -> Pos {
        let para = p.para.min(self.paragraphs.len() - 1);
        Pos::new(para, p.ch.min(self.paragraphs[para].len()))
    }

    /// Formatting that newly typed text at `p` should inherit.
    pub fn fmt_at(&self, p: Pos) -> Fmt {
        let para = &self.paragraphs[p.para];
        if p.ch > 0 {
            para.fmts[p.ch - 1]
        } else {
            para.fmts.first().copied().unwrap_or_default()
        }
    }

    /// Splits the paragraph at `p`. The new paragraph gets `next_style`.
    pub fn split(&mut self, p: Pos, next_style: &str) -> Pos {
        let para = &mut self.paragraphs[p.para];
        let mut tail = Paragraph::new(next_style);
        tail.chars = para.chars.split_off(p.ch);
        tail.fmts = para.fmts.split_off(p.ch);
        // Splitting at the very start keeps the style on the line that moves down.
        if p.ch == 0 {
            tail.style = para.style.clone();
        }
        self.paragraphs.insert(p.para + 1, tail);
        Pos::new(p.para + 1, 0)
    }

    /// Inserts text (which may contain newlines) and returns the position after it.
    pub fn insert(&mut self, mut p: Pos, text: &str, fmt: Fmt, next_style: &str) -> Pos {
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                p = self.split(p, next_style);
            }
            let para = &mut self.paragraphs[p.para];
            let chars: Vec<char> = line.chars().filter(|c| *c != '\r').collect();
            let n = chars.len();
            para.chars.splice(p.ch..p.ch, chars);
            para.fmts.splice(p.ch..p.ch, std::iter::repeat_n(fmt, n));
            p.ch += n;
        }
        p
    }

    /// Deletes everything between `a` and `b` (in either order).
    pub fn delete(&mut self, a: Pos, b: Pos) -> Pos {
        let (a, b) = (a.min(b), a.max(b));
        if a.para == b.para {
            let para = &mut self.paragraphs[a.para];
            para.chars.drain(a.ch..b.ch);
            para.fmts.drain(a.ch..b.ch);
        } else {
            let last = &self.paragraphs[b.para];
            let tail_chars = last.chars[b.ch..].to_vec();
            let tail_fmts = last.fmts[b.ch..].to_vec();
            self.paragraphs.drain(a.para + 1..=b.para);
            let first = &mut self.paragraphs[a.para];
            first.chars.truncate(a.ch);
            first.fmts.truncate(a.ch);
            first.chars.extend(tail_chars);
            first.fmts.extend(tail_fmts);
        }
        a
    }

    pub fn text(&self, a: Pos, b: Pos) -> String {
        let (a, b) = (a.min(b), a.max(b));
        let mut out = String::new();
        for i in a.para..=b.para {
            let para = &self.paragraphs[i];
            let start = if i == a.para { a.ch } else { 0 };
            let end = if i == b.para { b.ch } else { para.len() };
            out.extend(&para.chars[start..end]);
            if i != b.para {
                out.push('\n');
            }
        }
        out
    }

    /// Calls `f` on the formatting of every character in the range.
    pub fn for_each_fmt(&mut self, a: Pos, b: Pos, mut f: impl FnMut(&mut Fmt)) {
        let (a, b) = (a.min(b), a.max(b));
        for i in a.para..=b.para {
            let para = &mut self.paragraphs[i];
            let start = if i == a.para { a.ch } else { 0 };
            let end = if i == b.para { b.ch } else { para.len() };
            para.fmts[start..end].iter_mut().for_each(&mut f);
        }
    }

    /// True if every character in the range satisfies `pred`.
    pub fn all_fmt(&self, a: Pos, b: Pos, pred: impl Fn(&Fmt) -> bool) -> bool {
        let (a, b) = (a.min(b), a.max(b));
        (a.para..=b.para).all(|i| {
            let para = &self.paragraphs[i];
            let start = if i == a.para { a.ch } else { 0 };
            let end = if i == b.para { b.ch } else { para.len() };
            para.fmts[start..end].iter().all(&pred)
        })
    }

    pub fn word_left(&self, p: Pos) -> Pos {
        if p.ch == 0 {
            return if p.para == 0 {
                p
            } else {
                Pos::new(p.para - 1, self.paragraphs[p.para - 1].len())
            };
        }
        let chars = &self.paragraphs[p.para].chars;
        let mut i = p.ch;
        while i > 0 && !chars[i - 1].is_alphanumeric() {
            i -= 1;
        }
        while i > 0 && chars[i - 1].is_alphanumeric() {
            i -= 1;
        }
        Pos::new(p.para, i)
    }

    pub fn word_right(&self, p: Pos) -> Pos {
        let chars = &self.paragraphs[p.para].chars;
        if p.ch == chars.len() {
            return if p.para + 1 < self.paragraphs.len() {
                Pos::new(p.para + 1, 0)
            } else {
                p
            };
        }
        let mut i = p.ch;
        while i < chars.len() && !chars[i].is_alphanumeric() {
            i += 1;
        }
        while i < chars.len() && chars[i].is_alphanumeric() {
            i += 1;
        }
        Pos::new(p.para, i)
    }

    pub fn word_count(&self) -> usize {
        self.paragraphs
            .iter()
            .map(|p| {
                p.chars
                    .split(|c| c.is_whitespace())
                    .filter(|w| !w.is_empty())
                    .count()
            })
            .sum()
    }
}

// ---- File format ----------------------------------------------------------
//
// `.folio` files are plain JSON so they stay readable, diffable and easy for
// other tools to generate.

#[derive(Serialize, Deserialize)]
struct FileDoc {
    folio: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    theme: Option<String>,
    paragraphs: Vec<FilePara>,
}

#[derive(Serialize, Deserialize)]
struct FilePara {
    style: String,
    #[serde(default)]
    runs: Vec<FileRun>,
}

#[derive(Serialize, Deserialize)]
struct FileRun {
    text: String,
    #[serde(flatten)]
    fmt: Fmt,
}

impl Document {
    pub fn to_json(&self) -> String {
        let file = FileDoc {
            folio: 1,
            theme: self.theme.clone(),
            paragraphs: self
                .paragraphs
                .iter()
                .map(|p| FilePara {
                    style: p.style.clone(),
                    runs: p
                        .runs()
                        .into_iter()
                        .map(|(text, fmt)| FileRun { text, fmt })
                        .collect(),
                })
                .collect(),
        };
        serde_json::to_string_pretty(&file).expect("document serializes")
    }

    pub fn from_json(s: &str) -> Result<Self, String> {
        let file: FileDoc = serde_json::from_str(s).map_err(|e| e.to_string())?;
        let mut paragraphs: Vec<Paragraph> = file
            .paragraphs
            .into_iter()
            .map(|fp| {
                let mut p = Paragraph::new(&fp.style);
                for run in fp.runs {
                    for c in run.text.chars().filter(|c| *c != '\n' && *c != '\r') {
                        p.chars.push(c);
                        p.fmts.push(run.fmt);
                    }
                }
                p
            })
            .collect();
        if paragraphs.is_empty() {
            paragraphs.push(Paragraph::new("body"));
        }
        Ok(Self {
            paragraphs,
            theme: file.theme,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(doc: &Document) -> String {
        doc.text(Pos::default(), doc.end())
    }

    #[test]
    fn insert_split_delete() {
        let mut d = Document::default();
        let p = d.insert(Pos::default(), "hello\nworld", Fmt::default(), "body");
        assert_eq!(p, Pos::new(1, 5));
        assert_eq!(plain(&d), "hello\nworld");
        d.delete(Pos::new(0, 3), Pos::new(1, 2));
        assert_eq!(plain(&d), "helrld");
        assert_eq!(d.paragraphs.len(), 1);
    }

    #[test]
    fn json_round_trip_keeps_formatting() {
        let mut d = Document::default();
        d.insert(Pos::default(), "ab", Fmt::default(), "body");
        d.for_each_fmt(Pos::new(0, 1), Pos::new(0, 2), |f| f.bold = true);
        d.paragraphs[0].style = "h1".into();
        let back = Document::from_json(&d.to_json()).unwrap();
        assert_eq!(back.paragraphs[0].style, "h1");
        assert_eq!(back.paragraphs[0].runs().len(), 2);
        assert!(back.paragraphs[0].fmts[1].bold);
    }
}
