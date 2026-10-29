//! Copying inside text9000 keeps formatting, styles, list items and links.
//!
//! The system clipboard gets plain text, so pasting into other apps works
//! as before. Alongside it text9000 keeps the formatted copy in
//! `cache/clipboard.json` in its config folder, tagged with that plain text.
//! When a paste's text matches, text9000 uses the formatted copy, which also
//! works between text9000 windows (they're separate processes). Anything copied
//! elsewhere since won't match, and pastes as plain text.

use serde::{Deserialize, Serialize};

use crate::doc::Document;

#[derive(Serialize, Deserialize)]
struct Copied {
    text: String,
    /// The fragment as a .folio document.
    doc: String,
}

fn path() -> std::path::PathBuf {
    crate::settings::config_dir().join("cache/clipboard.json")
}

/// Remembers the formatted version of what was just copied.
pub fn remember(text: &str, fragment: &Document) {
    let copied = Copied {
        text: text.to_owned(),
        doc: fragment.to_json(),
    };
    let path = path();
    let _ = std::fs::create_dir_all(path.parent().expect("has parent"));
    let _ = std::fs::write(path, serde_json::to_string(&copied).expect("serializes"));
}

/// The formatted version of `pasted`, if it's what text9000 last copied.
pub fn formatted(pasted: &str) -> Option<Document> {
    let copied: Copied = serde_json::from_str(&std::fs::read_to_string(path()).ok()?).ok()?;
    // Some systems change line endings on the way through the clipboard.
    let same = copied.text == pasted || copied.text == pasted.replace("\r\n", "\n");
    same.then(|| Document::from_json(&copied.doc).ok()).flatten()
}
