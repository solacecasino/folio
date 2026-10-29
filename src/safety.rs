//! The safety net: version snapshots and crash recovery. Everything is kept in
//! the config folder as ordinary `.folio` files you can open yourself.
//!
//! - `versions/<doc id>/<timestamp>.folio`: a snapshot per save, newest N kept.
//! - `recovery/<pid>.json`: unsaved work, rewritten while you type and
//!   removed when you save or quit. One left behind means text9000 didn't quit
//!   cleanly, so it offers it back on the next launch.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::doc::{Document, Pos};
use crate::settings::config_dir;

/// A random-enough id for a new document.
pub fn new_doc_id() -> String {
    use std::hash::BuildHasher;
    let seed = (std::time::SystemTime::now(), std::process::id());
    format!("{:016x}", std::collections::hash_map::RandomState::new().hash_one(seed))
}

/// Local time as `2026-10-23 14:05:09`.
pub fn local_timestamp() -> String {
    let t = crate::platform::local_time();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    )
}

// ---- Versions ---------------------------------------------------------------

fn versions_dir(id: &str) -> PathBuf {
    config_dir().join("versions").join(id)
}

/// Saves a snapshot of `doc` and keeps only the newest `keep`.
pub fn save_version(doc: &Document, keep: usize) -> Result<(), String> {
    save_version_in(&versions_root(), doc, keep, &local_timestamp())
}

fn versions_root() -> PathBuf {
    config_dir().join("versions")
}

fn save_version_in(root: &Path, doc: &Document, keep: usize, stamp: &str) -> Result<(), String> {
    let (Some(id), true) = (&doc.id, keep > 0) else { return Ok(()) };
    let dir = root.join(id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Timestamps sort in order as file names.
    let name = stamp.replace([' ', ':'], "-");
    std::fs::write(dir.join(format!("{name}.{}", crate::series::EXTENSION)), doc.to_json()).map_err(|e| e.to_string())?;

    let mut files = document_files(&dir);
    files.sort();
    let excess = files.len().saturating_sub(keep);
    for old in &files[..excess] {
        let _ = std::fs::remove_file(old);
    }
    Ok(())
}

/// Versions are .t9k files; ones Folio kept are .folio, and both are listed.
fn document_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| crate::series::is_document(p))
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Clone, Debug)]
pub struct Version {
    pub path: PathBuf,
    /// e.g. "2026-10-23 14:05"
    pub when: String,
    pub words: usize,
    /// The start of the first non-empty paragraph.
    pub preview: String,
}

/// Saved versions of a document, newest first.
pub fn list_versions(id: &str) -> Vec<Version> {
    list_versions_in(&versions_dir(id))
}

fn list_versions_in(dir: &Path) -> Vec<Version> {
    let mut files = document_files(dir);
    files.sort();
    files.reverse();
    files
        .into_iter()
        .filter_map(|path| {
            let doc = Document::from_json(&std::fs::read_to_string(&path).ok()?).ok()?;
            let stem = path.file_stem()?.to_string_lossy().into_owned();
            // "2026-10-23-14-05-09" → "2026-10-23 14:05"
            let when = match stem.get(..16) {
                Some(s) if s.len() == 16 => format!("{} {}:{}", &s[..10], &s[11..13], &s[14..16]),
                _ => stem,
            };
            let preview: String = doc
                .paragraphs
                .iter()
                .find(|p| !p.chars.iter().all(|c| c.is_whitespace()))
                .map(|p| p.chars.iter().take(48).collect())
                .unwrap_or_default();
            Some(Version {
                path,
                when,
                words: doc.word_count(),
                preview,
            })
        })
        .collect()
}

// ---- Recovery ---------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub struct Recovery {
    /// Where the document was saved, if it ever was.
    pub path: Option<PathBuf>,
    pub saved_at: String,
    /// The document, as in a `.folio` file.
    pub doc: serde_json::Value,
}

fn recovery_dir() -> PathBuf {
    config_dir().join("recovery")
}

fn own_recovery_file() -> PathBuf {
    recovery_dir().join(format!("{}.json", std::process::id()))
}

/// Writes this window's unsaved work, replacing the last copy.
pub fn write_recovery(doc: &Document, path: Option<&Path>) -> Result<(), String> {
    let record = Recovery {
        path: path.map(Path::to_path_buf),
        saved_at: local_timestamp(),
        doc: serde_json::from_str(&doc.to_json()).map_err(|e| e.to_string())?,
    };
    std::fs::create_dir_all(recovery_dir()).map_err(|e| e.to_string())?;
    let tmp = own_recovery_file().with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string(&record).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    // Rename so a crash mid-write never leaves a half-written file.
    std::fs::rename(tmp, own_recovery_file()).map_err(|e| e.to_string())
}

pub fn clear_recovery() {
    let _ = std::fs::remove_file(own_recovery_file());
}

/// Recovery files left by copies of text9000 that are no longer running, newest first.
pub fn orphaned_recoveries() -> Vec<(PathBuf, Recovery)> {
    let Ok(entries) = std::fs::read_dir(recovery_dir()) else { return Vec::new() };
    let mut found: Vec<(PathBuf, Recovery)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter(|p| {
            // Another text9000 window that's still open owns its own file.
            let pid: Option<u32> = p.file_stem().and_then(|s| s.to_str()?.parse().ok());
            pid.is_none_or(|pid| pid != std::process::id() && !crate::platform::process_alive(pid))
        })
        .filter_map(|p| {
            let record = serde_json::from_str(&std::fs::read_to_string(&p).ok()?).ok()?;
            Some((p, record))
        })
        .collect();
    found.sort_by(|a, b| b.1.saved_at.cmp(&a.1.saved_at));
    found
}

impl Recovery {
    pub fn document(&self) -> Result<Document, String> {
        Document::from_json(&self.doc.to_string())
    }

    pub fn describe(&self) -> String {
        let doc = self.document().ok();
        let words = doc.as_ref().map_or(0, Document::word_count);
        let preview: String = doc
            .map(|d| d.text(Pos::default(), d.end()).chars().take(60).collect::<String>())
            .unwrap_or_default();
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or("an untitled document".to_owned(), |n| format!("“{}”", n.to_string_lossy()));
        format!(
            "text9000 closed unexpectedly while you were working on {name} ({words} words, last kept {}).\n\n“{}…”\n\nRestore it?",
            self.saved_at,
            preview.trim()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Fmt;

    #[test]
    fn versions_are_kept_newest_first_and_capped() {
        let root = std::env::temp_dir().join(format!("text9000-versions-{}", new_doc_id()));
        let mut doc = Document {
            id: Some("abc".into()),
            ..Default::default()
        };
        for (i, word) in ["one", "two", "three", "four"].iter().enumerate() {
            doc.paragraphs[0] = crate::doc::Paragraph::new("body");
            doc.insert(Pos::default(), word, Fmt::default(), "body");
            save_version_in(&root, &doc, 3, &format!("2026-10-23 10:00:0{i}")).unwrap();
        }
        let versions = list_versions_in(&root.join("abc"));
        assert_eq!(versions.len(), 3, "only the newest 3 are kept");
        assert_eq!(versions[0].preview, "four");
        assert_eq!(versions[2].preview, "two");
        assert_eq!(versions[0].when, "2026-10-23 10:00");

        let restored = Document::from_json(&std::fs::read_to_string(&versions[1].path).unwrap()).unwrap();
        assert_eq!(restored.text(Pos::default(), restored.end()), "three");
        assert_eq!(restored.id.as_deref(), Some("abc"), "the id survives");

        let unsaved = Document::default();
        save_version_in(&root, &unsaved, 3, "x").unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn ids_differ() {
        assert_ne!(new_doc_id(), new_doc_id());
    }
}
