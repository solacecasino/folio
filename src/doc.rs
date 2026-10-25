//! The document model: a list of styled paragraphs, each holding characters
//! with per-character formatting. Kept independent of the UI so plugins and
//! exporters can work on it directly later.

use std::sync::Mutex;

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
    /// The link this character is part of. Files store the URL itself.
    #[serde(skip)]
    pub link: Option<Link>,
}

/// A link target. URLs are kept in one table so formatting stays a small
/// `Copy` value per character; the same URL always gets the same `Link`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Link(u32);

static LINKS: Mutex<Vec<String>> = Mutex::new(Vec::new());

impl Link {
    pub fn new(url: &str) -> Self {
        let mut links = LINKS.lock().unwrap_or_else(|e| e.into_inner());
        let i = links.iter().position(|u| u == url).unwrap_or_else(|| {
            links.push(url.to_owned());
            links.len() - 1
        });
        Link(i as u32)
    }

    pub fn url(self) -> String {
        LINKS.lock().unwrap_or_else(|e| e.into_inner())[self.0 as usize].clone()
    }
}

/// A paragraph that's a list item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct List {
    pub kind: ListKind,
    /// Nesting depth, 0 for the outermost level.
    #[serde(default)]
    pub level: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ListKind {
    Bullet,
    Numbered,
}

/// Deepest list nesting.
pub const MAX_LIST_LEVEL: u8 = 8;

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
    /// Set when the paragraph is a list item.
    pub list: Option<List>,
}

impl Paragraph {
    pub fn new(style: &str) -> Self {
        Self {
            style: style.to_owned(),
            chars: Vec::new(),
            fmts: Vec::new(),
            list: None,
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
    /// Stable id, given on first save, so version history follows the
    /// document when it's renamed or moved.
    pub id: Option<String>,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            paragraphs: vec![Paragraph::new("body")],
            theme: None,
            id: None,
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
        // A list continues onto the new line.
        tail.list = para.list;
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

    /// The range `a..b` as its own document, keeping formatting, styles,
    /// list items and links.
    pub fn fragment(&self, a: Pos, b: Pos) -> Document {
        let (a, b) = (a.min(b), a.max(b));
        let paragraphs = (a.para..=b.para)
            .map(|i| {
                let para = &self.paragraphs[i];
                let start = if i == a.para { a.ch } else { 0 };
                let end = if i == b.para { b.ch } else { para.len() };
                Paragraph {
                    style: para.style.clone(),
                    chars: para.chars[start..end].to_vec(),
                    fmts: para.fmts[start..end].to_vec(),
                    list: para.list,
                }
            })
            .collect();
        Document {
            paragraphs,
            theme: None,
            id: None,
        }
    }

    /// Inserts a fragment at `p` and returns the position after it. Its
    /// first and last paragraphs join the text around `p`; the ones between
    /// come in whole, with their styles and list items. Pasted at the start
    /// of an empty paragraph, the first one brings its style too.
    pub fn insert_fragment(&mut self, p: Pos, fragment: &Document) -> Pos {
        let Some((first, rest)) = fragment.paragraphs.split_first() else {
            return p;
        };
        let here = &mut self.paragraphs[p.para];
        if here.chars.is_empty() {
            here.style = first.style.clone();
            here.list = first.list;
        }
        let tail_chars = here.chars.split_off(p.ch);
        let tail_fmts = here.fmts.split_off(p.ch);
        here.chars.extend(&first.chars);
        here.fmts.extend(&first.fmts);
        let mut at = Pos::new(p.para, here.chars.len());
        for (k, para) in rest.iter().enumerate() {
            self.paragraphs.insert(p.para + 1 + k, para.clone());
            at = Pos::new(p.para + 1 + k, para.chars.len());
        }
        let last = &mut self.paragraphs[at.para];
        last.chars.extend(tail_chars);
        last.fmts.extend(tail_fmts);
        at
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

    /// Every match of `query` (within single paragraphs), in document order.
    pub fn find_all(&self, query: &str, match_case: bool, whole_word: bool) -> Vec<(Pos, Pos)> {
        let fold = |c: char| if match_case { c } else { c.to_lowercase().next().unwrap_or(c) };
        let needle: Vec<char> = query.chars().map(fold).collect();
        if needle.is_empty() || needle.contains(&'\n') {
            return Vec::new();
        }
        let mut found = Vec::new();
        for (i, para) in self.paragraphs.iter().enumerate() {
            let hay: Vec<char> = para.chars.iter().copied().map(fold).collect();
            let mut start = 0;
            while start + needle.len() <= hay.len() {
                let end = start + needle.len();
                let is_word = |k: Option<&char>| k.is_some_and(|c| c.is_alphanumeric());
                let bounded = !whole_word
                    || (!is_word(start.checked_sub(1).and_then(|k| hay.get(k))) && !is_word(hay.get(end)));
                if hay[start..end] == needle[..] && bounded {
                    found.push((Pos::new(i, start), Pos::new(i, end)));
                    start = end;
                } else {
                    start += 1;
                }
            }
        }
        found
    }

    /// Makes paragraphs `from..=to` list items (or plain paragraphs with None).
    pub fn set_list(&mut self, from: usize, to: usize, list: Option<List>) {
        let last = self.paragraphs.len() - 1;
        for p in &mut self.paragraphs[from.min(last)..=to.min(last)] {
            p.list = list;
        }
    }

    /// Links the range to `url`, or removes links in it with None.
    pub fn set_link(&mut self, a: Pos, b: Pos, url: Option<&str>) {
        let link = url.map(Link::new);
        self.for_each_fmt(a, b, |f| f.link = link);
    }

    /// The whole link around a position, if there's one: its range and URL.
    pub fn link_at(&self, p: Pos) -> Option<(Pos, Pos, Link)> {
        let fmts = &self.paragraphs[p.para].fmts;
        // The character at the position, or the one before it at a run's end.
        let i = [p.ch, p.ch.wrapping_sub(1)].into_iter().find(|&i| fmts.get(i).is_some_and(|f| f.link.is_some()))?;
        let link = fmts[i].link?;
        let start = (0..=i).rev().take_while(|&k| fmts[k].link == Some(link)).last().unwrap_or(i);
        let end = (i..fmts.len()).take_while(|&k| fmts[k].link == Some(link)).last().unwrap_or(i) + 1;
        Some((Pos::new(p.para, start), Pos::new(p.para, end), link))
    }

    /// The number shown for each numbered list item (1, 2, 3 ...), counting
    /// per level and starting again after anything that isn't a list item.
    pub fn list_numbers(&self) -> Vec<Option<u32>> {
        let mut counters = [0u32; MAX_LIST_LEVEL as usize + 1];
        self.paragraphs
            .iter()
            .map(|p| match p.list {
                None => {
                    counters = [0; MAX_LIST_LEVEL as usize + 1];
                    None
                }
                Some(List { kind, level }) => {
                    let level = level.min(MAX_LIST_LEVEL) as usize;
                    // Going back up a level starts deeper levels over.
                    counters[level + 1..].iter_mut().for_each(|c| *c = 0);
                    match kind {
                        ListKind::Numbered => {
                            counters[level] += 1;
                            Some(counters[level])
                        }
                        ListKind::Bullet => {
                            counters[level] = 0;
                            None
                        }
                    }
                }
            })
            .collect()
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
    id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    theme: Option<String>,
    paragraphs: Vec<FilePara>,
}

#[derive(Serialize, Deserialize)]
struct FilePara {
    style: String,
    /// `{ "kind": "bullet" | "numbered", "level": 0 }` for list items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    list: Option<List>,
    #[serde(default)]
    runs: Vec<FileRun>,
}

#[derive(Serialize, Deserialize)]
struct FileRun {
    text: String,
    #[serde(flatten)]
    fmt: Fmt,
    /// The URL of a linked run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    link: Option<String>,
}

impl Document {
    pub fn to_json(&self) -> String {
        let file = FileDoc {
            folio: 1,
            id: self.id.clone(),
            theme: self.theme.clone(),
            paragraphs: self
                .paragraphs
                .iter()
                .map(|p| FilePara {
                    style: p.style.clone(),
                    list: p.list,
                    runs: p
                        .runs()
                        .into_iter()
                        .map(|(text, fmt)| FileRun {
                            text,
                            link: fmt.link.map(Link::url),
                            fmt,
                        })
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
                p.list = fp.list.map(|l| List {
                    level: l.level.min(MAX_LIST_LEVEL),
                    ..l
                });
                for run in fp.runs {
                    let fmt = Fmt {
                        link: run.link.as_deref().filter(|u| !u.is_empty()).map(Link::new),
                        ..run.fmt
                    };
                    for c in run.text.chars().filter(|c| *c != '\n' && *c != '\r') {
                        p.chars.push(c);
                        p.fmts.push(fmt);
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
            id: file.id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragments_keep_formatting_lists_and_links() {
        let mut doc = Document::default();
        doc.insert(Pos::default(), "Title\nFirst item\nSecond item\nPlain", Fmt::default(), "body");
        doc.paragraphs[0].style = "h1".into();
        doc.set_list(1, 2, Some(List { kind: ListKind::Bullet, level: 0 }));
        doc.for_each_fmt(Pos::new(1, 0), Pos::new(1, 5), |f| f.bold = true);
        doc.set_link(Pos::new(2, 0), Pos::new(2, 6), Some("https://x.y"));

        // From the middle of the title to the middle of the second item.
        let frag = doc.fragment(Pos::new(0, 2), Pos::new(2, 6));
        assert_eq!(frag.paragraphs.len(), 3);
        let back = Document::from_json(&frag.to_json()).unwrap();
        assert_eq!(back.paragraphs[1].list.map(|l| l.kind), Some(ListKind::Bullet));
        assert!(back.paragraphs[1].fmts[0].bold);
        assert_eq!(back.paragraphs[2].fmts[0].link.map(Link::url).as_deref(), Some("https://x.y"));

        // Into the middle of "Plain": its first and last parts join the text there.
        let mut target = Document::default();
        target.insert(Pos::default(), "Plain", Fmt::default(), "body");
        let end = target.insert_fragment(Pos::new(0, 2), &back);
        let text = target.text(Pos::default(), target.end());
        assert_eq!(text, "Pltle\nFirst item\nSecondain");
        assert_eq!(end, Pos::new(2, 6));
        assert!(target.paragraphs[1].list.is_some() && target.paragraphs[1].fmts[0].bold);
        assert!(target.paragraphs[0].list.is_none(), "the paragraph pasted into keeps its own kind");

        // At the start of an empty paragraph, the first paragraph's style comes along.
        let mut empty = Document::default();
        empty.insert_fragment(Pos::default(), &doc.fragment(Pos::new(0, 0), Pos::new(1, 3)));
        assert_eq!(empty.paragraphs[0].style, "h1");
        assert_eq!(empty.paragraphs[1].list.map(|l| l.kind), Some(ListKind::Bullet));
    }

    #[test]
    fn lists_and_links_round_trip_and_number_themselves() {
        let mut doc = Document::default();
        let end = doc.insert(Pos::default(), "Intro\nOne\nTwo\nDeeper\nThree\nAfter", Fmt::default(), "body");
        assert_eq!(end.para, 5);
        let numbered = |level| Some(List { kind: ListKind::Numbered, level });
        doc.set_list(1, 2, numbered(0));
        doc.set_list(3, 3, Some(List { kind: ListKind::Bullet, level: 1 }));
        doc.set_list(4, 4, numbered(0));
        assert_eq!(doc.list_numbers(), [None, Some(1), Some(2), None, Some(3), None], "a nested item doesn't restart its parent");

        doc.set_link(Pos::new(0, 0), Pos::new(0, 5), Some("https://example.com"));
        let (a, b, link) = doc.link_at(Pos::new(0, 2)).unwrap();
        assert_eq!((a, b, link.url().as_str()), (Pos::new(0, 0), Pos::new(0, 5), "https://example.com"));
        assert!(doc.link_at(Pos::new(0, 5)).is_some(), "right after a link still counts");
        assert!(doc.link_at(Pos::new(1, 1)).is_none());

        let back = Document::from_json(&doc.to_json()).unwrap();
        assert_eq!(back.paragraphs[2].list, numbered(0));
        assert_eq!(back.paragraphs[3].list.unwrap().level, 1);
        assert_eq!(back.paragraphs[0].fmts[0].link.map(Link::url).as_deref(), Some("https://example.com"));
        assert!(back.paragraphs[5].list.is_none());
        assert!(doc.to_json().contains("\"link\": \"https://example.com\""), "files keep the URL itself");

        // Enter in a list item continues the list.
        let p = doc.split(Pos::new(2, 3), "body");
        assert_eq!(doc.paragraphs[p.para].list, numbered(0));
        // Older files without lists or links still load.
        let old = Document::from_json(r#"{"folio":1,"paragraphs":[{"style":"body","runs":[{"text":"Hi","bold":true}]}]}"#).unwrap();
        assert!(old.paragraphs[0].list.is_none() && old.paragraphs[0].fmts[0].link.is_none());
    }

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
    fn find_respects_case_and_whole_words() {
        let mut d = Document::default();
        d.insert(Pos::default(), "Cat catalog cat\nthe CAT sat", Fmt::default(), "body");
        assert_eq!(d.find_all("cat", false, false).len(), 4);
        assert_eq!(d.find_all("cat", true, false).len(), 2, "cat in catalog and the last cat");
        let whole = d.find_all("cat", false, true);
        assert_eq!(whole, vec![
            (Pos::new(0, 0), Pos::new(0, 3)),
            (Pos::new(0, 12), Pos::new(0, 15)),
            (Pos::new(1, 4), Pos::new(1, 7)),
        ]);
        assert!(d.find_all("", false, false).is_empty());
        assert_eq!(d.find_all("aa", false, false).len(), 0);
        let mut d = Document::default();
        d.insert(Pos::default(), "aaaa", Fmt::default(), "body");
        assert_eq!(d.find_all("aa", false, false).len(), 2, "matches don't overlap");
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
