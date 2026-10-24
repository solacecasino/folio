//! The shared document: one `LoroText` in the Quill model.
//!
//! Paragraphs are separated by `\n`, with a trailing `\n`. Inline formatting is
//! the marks `bold`, `italic` and `underline`; a paragraph's style ("body",
//! "h1", ...) is a `style` mark on that paragraph's `\n`.
//!
//! Folio sends whole-document snapshots. Each one is turned into CRDT edits by
//! diffing it against the version Folio last had (the "shadow"), so a snapshot
//! typed before a remote edit reached Folio never reverts that remote edit.

use loro::cursor::{Cursor, Side};
use loro::{
    ExpandType, ExportMode, Frontiers, LoroDoc, LoroText, LoroValue, StyleConfig, StyleConfigMap,
    TextDelta, UndoManager, UpdateOptions, VersionVector,
};
use serde::{Deserialize, Serialize};

const TEXT: &str = "text";
const INLINE: [&str; 3] = ["bold", "italic", "underline"];
/// Above this many characters, diff by line instead of by character.
const CHAR_DIFF_LIMIT: usize = 50_000;

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub text: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub underline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Paragraph {
    #[serde(default = "body")]
    pub style: String,
    #[serde(default)]
    pub runs: Vec<Run>,
}

fn body() -> String {
    "body".into()
}

/// A caret position as Folio counts it: paragraph index and character index.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pos {
    pub para: usize,
    pub ch: usize,
}

/// Formatting of one character. `style` is only meaningful on `\n`, the
/// inline flags only on other characters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Attrs {
    flags: [bool; 3],
    style: Option<String>,
}

/// The document as one string plus per-character formatting.
fn flatten(paragraphs: &[Paragraph]) -> (String, Vec<Attrs>) {
    let mut text = String::new();
    let mut attrs = Vec::new();
    for p in paragraphs {
        for run in &p.runs {
            for c in run.text.chars().filter(|&c| c != '\n') {
                text.push(c);
                attrs.push(Attrs {
                    flags: [run.bold, run.italic, run.underline],
                    style: None,
                });
            }
        }
        text.push('\n');
        attrs.push(Attrs {
            flags: [false; 3],
            style: Some(p.style.clone()),
        });
    }
    (text, attrs)
}

fn unflatten(chars: &[char], attrs: &[Attrs]) -> Vec<Paragraph> {
    let mut out = Vec::new();
    let mut runs: Vec<Run> = Vec::new();
    for (&c, a) in chars.iter().zip(attrs) {
        if c == '\n' {
            out.push(Paragraph {
                style: a.style.clone().unwrap_or_else(body),
                runs: std::mem::take(&mut runs),
            });
            continue;
        }
        let [bold, italic, underline] = a.flags;
        match runs.last_mut() {
            Some(r) if (r.bold, r.italic, r.underline) == (bold, italic, underline) => {
                r.text.push(c)
            }
            _ => runs.push(Run {
                text: c.to_string(),
                bold,
                italic,
                underline,
            }),
        }
    }
    // Text after the last `\n` can only come from concurrent edits; keep it.
    if !runs.is_empty() || out.is_empty() {
        out.push(Paragraph {
            style: body(),
            runs,
        });
    }
    out
}

/// Reads the text's characters and their formatting.
fn read(text: &LoroText) -> (Vec<char>, Vec<Attrs>) {
    let mut chars = Vec::new();
    let mut attrs = Vec::new();
    for d in text.to_delta() {
        let TextDelta::Insert { insert, attributes } = d else {
            continue;
        };
        let get = |k: &str| attributes.as_ref().and_then(|m| m.get(k));
        let a = Attrs {
            flags: INLINE.map(|k| matches!(get(k), Some(LoroValue::Bool(true)))),
            style: match get("style") {
                Some(LoroValue::String(s)) => Some(s.to_string()),
                _ => None,
            },
        };
        for c in insert.chars() {
            chars.push(c);
            attrs.push(a.clone());
        }
    }
    (chars, attrs)
}

fn configure(doc: &LoroDoc) {
    let mut styles = StyleConfigMap::new();
    for k in INLINE {
        styles.insert(
            k.into(),
            StyleConfig {
                expand: ExpandType::After,
            },
        );
    }
    styles.insert(
        "style".into(),
        StyleConfig {
            expand: ExpandType::None,
        },
    );
    doc.config_text_style(styles);
}

/// Turns `text` into exactly `paragraphs`: a text diff, then mark fixes
/// wherever formatting differs. Does not commit.
fn apply(text: &LoroText, paragraphs: &[Paragraph]) {
    let (target, want) = flatten(paragraphs);
    if text.to_string() != target {
        let options = UpdateOptions {
            timeout_ms: Some(200.0),
            use_refined_diff: true,
        };
        let done = if target.chars().count() > CHAR_DIFF_LIMIT {
            text.update_by_line(&target, options)
        } else {
            text.update(&target, options)
        };
        if done.is_err() {
            // The diff timed out; a coarse line diff is still correct.
            let _ = text.update_by_line(&target, UpdateOptions::default());
        }
    }
    let (chars, have) = read(text);
    debug_assert_eq!(chars.len(), want.len());

    // Ranges where a key must change, as (start, end, new value).
    let mut fixes: Vec<(usize, usize, &str, Option<LoroValue>)> = Vec::new();
    let mut push = |i: usize, key: &'static str, value: Option<LoroValue>| match fixes.last_mut() {
        Some((_, end, k, v)) if *end == i && *k == key && *v == value => *end += 1,
        _ => fixes.push((i, i + 1, key, value)),
    };
    for (key_index, key) in INLINE.iter().enumerate() {
        for (i, c) in chars.iter().enumerate() {
            let (w, h) = (want[i].flags[key_index], have[i].flags[key_index]);
            if *c != '\n' && w != h {
                push(i, key, w.then_some(LoroValue::Bool(true)));
            }
        }
    }
    for (i, c) in chars.iter().enumerate() {
        if *c == '\n' && want[i].style != have[i].style {
            push(i, "style", want[i].style.clone().map(|s| s.into()));
        }
    }
    for (start, end, key, value) in fixes {
        let _ = match value {
            Some(v) => text.mark(start..end, key, v),
            None => text.unmark(start..end, key),
        };
    }
}

pub struct Crdt {
    doc: LoroDoc,
    text: LoroText,
    undo: UndoManager,
    /// The version whose content Folio has on screen.
    shadow: Frontiers,
    /// Everything up to here has been handed to the network.
    sent: VersionVector,
}

impl Default for Crdt {
    fn default() -> Self {
        Self::new()
    }
}

impl Crdt {
    pub fn new() -> Self {
        let doc = LoroDoc::new();
        configure(&doc);
        let text = doc.get_text(TEXT);
        let mut undo = UndoManager::new(&doc);
        undo.set_merge_interval(500);
        Self {
            shadow: doc.state_frontiers(),
            sent: doc.oplog_vv(),
            doc,
            text,
            undo,
        }
    }

    /// A new document holding `paragraphs`, already in sync with Folio.
    pub fn from_paragraphs(paragraphs: &[Paragraph]) -> Self {
        let mut crdt = Self::new();
        let empty = [Paragraph {
            style: body(),
            runs: Vec::new(),
        }];
        apply(
            &crdt.text,
            if paragraphs.is_empty() {
                &empty
            } else {
                paragraphs
            },
        );
        crdt.doc.commit();
        crdt.shadow = crdt.doc.state_frontiers();
        crdt.sent = crdt.doc.oplog_vv();
        // The starting text is not something to undo.
        crdt.undo = UndoManager::new(&crdt.doc);
        crdt.undo.set_merge_interval(500);
        crdt
    }

    /// A replica of a host's document. Folio does not have it yet.
    pub fn from_snapshot(bytes: &[u8]) -> Result<Self, String> {
        let crdt = Self::new();
        crdt.doc.import(bytes).map_err(|e| e.to_string())?;
        let mut crdt = crdt;
        crdt.sent = crdt.doc.oplog_vv();
        Ok(crdt)
    }

    pub fn paragraphs(&self) -> Vec<Paragraph> {
        let (chars, attrs) = read(&self.text);
        unflatten(&chars, &attrs)
    }

    /// Folio's snapshot: records the user's edits since the shadow version.
    pub fn apply_local(&mut self, paragraphs: &[Paragraph]) {
        if self.shadow == self.doc.state_frontiers() {
            apply(&self.text, paragraphs);
            self.doc.commit();
            self.shadow = self.doc.state_frontiers();
            return;
        }
        // Folio typed on top of an older version: diff against that version
        // on a fork, then bring the fork's edits in. They merge with the
        // newer remote edits instead of undoing them.
        let Ok(fork) = self.doc.fork_at(&self.shadow) else {
            apply(&self.text, paragraphs);
            self.doc.commit();
            self.shadow = self.doc.state_frontiers();
            return;
        };
        configure(&fork);
        apply(&fork.get_text(TEXT), paragraphs);
        fork.commit();
        if let Ok(bytes) = fork.export(ExportMode::updates(&self.doc.oplog_vv())) {
            let _ = self.doc.import(&bytes);
        }
        self.shadow = fork.state_frontiers();
    }

    /// True when the CRDT has changes Folio has not seen.
    pub fn folio_behind(&self) -> bool {
        self.shadow != self.doc.state_frontiers()
    }

    pub fn frontier(&self) -> Frontiers {
        self.doc.state_frontiers()
    }

    /// Folio now shows the content of `frontier`.
    pub fn set_shadow(&mut self, frontier: Frontiers) {
        self.shadow = frontier;
    }

    pub fn undo(&mut self) -> bool {
        self.undo.undo().unwrap_or(false)
    }

    pub fn redo(&mut self) -> bool {
        self.undo.redo().unwrap_or(false)
    }

    /// The whole document, for a new guest.
    pub fn export_snapshot(&self) -> Vec<u8> {
        self.doc.export(ExportMode::Snapshot).unwrap_or_default()
    }

    /// Local changes not yet handed to the network.
    pub fn take_updates(&mut self) -> Option<Vec<u8>> {
        let vv = self.doc.oplog_vv();
        if vv == self.sent {
            return None;
        }
        let bytes = self.doc.export(ExportMode::updates(&self.sent)).ok();
        self.sent = vv;
        bytes
    }

    /// Changes from another peer. Call `take_updates` first so local changes
    /// are not mistaken for already sent.
    pub fn import(&mut self, bytes: &[u8]) -> Result<(), String> {
        let before = self.doc.oplog_vv();
        self.doc.import(bytes).map_err(|e| e.to_string())?;
        if before == self.sent {
            self.sent = self.doc.oplog_vv();
        }
        Ok(())
    }

    fn index_of(&self, pos: Pos) -> usize {
        let mut index = 0;
        let mut para = 0;
        for c in self.text.to_string().chars() {
            if para == pos.para {
                break;
            }
            index += 1;
            if c == '\n' {
                para += 1;
            }
        }
        let line_len = self
            .text
            .to_string()
            .chars()
            .skip(index)
            .take_while(|&c| c != '\n')
            .count();
        (index + pos.ch.min(line_len)).min(self.text.len_unicode())
    }

    fn pos_of(&self, index: usize) -> Pos {
        let mut pos = Pos::default();
        for c in self.text.to_string().chars().take(index) {
            if c == '\n' {
                pos.para += 1;
                pos.ch = 0;
            } else {
                pos.ch += 1;
            }
        }
        pos
    }

    /// A position that follows the text it sits in through later edits.
    pub fn cursor_at(&self, pos: Pos) -> Option<Cursor> {
        self.text.get_cursor(self.index_of(pos), Side::Middle)
    }

    pub fn resolve(&self, cursor: &Cursor) -> Option<Pos> {
        let index = self.doc.get_cursor_pos(cursor).ok()?.current.pos;
        Some(self.pos_of(index))
    }

    /// Where `before` and the current text first differ, from the end: the
    /// natural caret after an undo or redo.
    pub fn change_end(&self, before: &[Paragraph]) -> Pos {
        let (old, _) = flatten(before);
        let old: Vec<char> = old.chars().collect();
        let new: Vec<char> = self.text.to_string().chars().collect();
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(new[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        self.pos_of(new.len() - suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn para(style: &str, runs: &[(&str, bool, bool)]) -> Paragraph {
        Paragraph {
            style: style.into(),
            runs: runs
                .iter()
                .map(|&(text, bold, italic)| Run {
                    text: text.into(),
                    bold,
                    italic,
                    underline: false,
                })
                .collect(),
        }
    }

    fn sync(a: &mut Crdt, b: &mut Crdt) {
        if let Some(u) = a.take_updates() {
            b.import(&u).unwrap();
        }
        if let Some(u) = b.take_updates() {
            a.import(&u).unwrap();
        }
    }

    #[test]
    fn round_trips_paragraphs() {
        let doc = vec![
            para("h1", &[("Title", false, false)]),
            para("body", &[("Hi ", true, false), ("there", false, true)]),
            para("body", &[]),
        ];
        assert_eq!(Crdt::from_paragraphs(&doc).paragraphs(), doc);
    }

    #[test]
    fn concurrent_snapshots_converge_with_marks_and_styles() {
        let start = vec![
            para("h1", &[("Title", false, false)]),
            para("body", &[("Hello world", false, false)]),
        ];
        let mut a = Crdt::from_paragraphs(&start);
        let mut b = Crdt::from_snapshot(&a.export_snapshot()).unwrap();
        b.set_shadow(b.frontier());
        assert_eq!(b.paragraphs(), start);

        // A bolds "Hello" and makes the title an h2; B appends a paragraph
        // and italicizes "world", at the same time.
        a.apply_local(&[
            para("h2", &[("Title", false, false)]),
            para("body", &[("Hello", true, false), (" world", false, false)]),
        ]);
        b.apply_local(&[
            para("h1", &[("Title", false, false)]),
            para("body", &[("Hello ", false, false), ("world", false, true)]),
            para("quote", &[("New", false, false)]),
        ]);
        sync(&mut a, &mut b);

        let want = vec![
            para("h2", &[("Title", false, false)]),
            para(
                "body",
                &[
                    ("Hello", true, false),
                    (" ", false, false),
                    ("world", false, true),
                ],
            ),
            para("quote", &[("New", false, false)]),
        ];
        assert_eq!(a.paragraphs(), want);
        assert_eq!(b.paragraphs(), want);
    }

    #[test]
    fn stale_snapshot_does_not_revert_remote_edit() {
        let start = vec![para("body", &[("abc", false, false)])];
        let mut a = Crdt::from_paragraphs(&start);
        let mut b = Crdt::from_snapshot(&a.export_snapshot()).unwrap();
        b.set_shadow(b.frontier());

        // B types at the end; A receives it but Folio hasn't merged it yet
        // when A's user types at the start.
        b.apply_local(&[para("body", &[("abcX", false, false)])]);
        sync(&mut a, &mut b);
        assert!(a.folio_behind());
        a.apply_local(&[para("body", &[("Yabc", false, false)])]);
        sync(&mut a, &mut b);

        let want = vec![para("body", &[("YabcX", false, false)])];
        assert_eq!(a.paragraphs(), want);
        assert_eq!(b.paragraphs(), want);
    }

    #[test]
    fn undo_is_local_only() {
        let start = vec![para("body", &[("one", false, false)])];
        let mut a = Crdt::from_paragraphs(&start);
        let mut b = Crdt::from_snapshot(&a.export_snapshot()).unwrap();
        b.set_shadow(b.frontier());

        a.apply_local(&[para("body", &[("one two", false, false)])]);
        sync(&mut a, &mut b);
        b.set_shadow(b.frontier());
        b.apply_local(&[para("body", &[("zero one two", false, false)])]);
        sync(&mut a, &mut b);

        assert!(a.undo());
        assert_eq!(
            a.paragraphs(),
            vec![para("body", &[("zero one", false, false)])]
        );
        assert!(a.redo());
        assert_eq!(
            a.paragraphs(),
            vec![para("body", &[("zero one two", false, false)])]
        );
    }

    #[test]
    fn cursors_follow_remote_edits() {
        let start = vec![para("body", &[("hello", false, false)]), para("body", &[])];
        let mut a = Crdt::from_paragraphs(&start);
        let mut b = Crdt::from_snapshot(&a.export_snapshot()).unwrap();
        b.set_shadow(b.frontier());
        let caret = a.cursor_at(Pos { para: 0, ch: 3 }).unwrap();
        b.apply_local(&[
            para("body", &[("oh, hello", false, false)]),
            para("body", &[]),
        ]);
        sync(&mut a, &mut b);
        assert_eq!(a.resolve(&caret), Some(Pos { para: 0, ch: 7 }));
        assert_eq!(
            a.resolve(&a.cursor_at(Pos { para: 1, ch: 0 }).unwrap()),
            Some(Pos { para: 1, ch: 0 })
        );
    }
}
