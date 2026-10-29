//! What text9000 remembers between runs that isn't a preference: the window's
//! size and place, and recently opened documents. Kept in `state.json` in
//! the config folder, apart from the settings people edit by hand.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Documents in File → Open Recent.
const RECENT: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowRect {
    /// Top left of the window on screen, in points.
    pub x: f32,
    pub y: f32,
    /// Size of the window's content, in points.
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub window: Option<WindowRect>,
    pub recent: Vec<PathBuf>,
}

fn path() -> PathBuf {
    crate::settings::config_dir().join("state.json")
}

impl State {
    /// The saved state; documents that no longer exist are left out.
    pub fn load() -> Self {
        let mut state: State = std::fs::read_to_string(path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        state.recent.retain(|p| p.is_file());
        // A window from a screen that's gone, or a broken file, is ignored.
        state.window = state.window.filter(|w| {
            w.width >= 200.0 && w.height >= 200.0 && w.x > -10_000.0 && w.y > -10_000.0 && w.x < 20_000.0 && w.y < 20_000.0
        });
        state
    }

    pub fn save(&self) {
        let path = path();
        let _ = std::fs::create_dir_all(path.parent().expect("has parent"));
        let _ = std::fs::write(path, serde_json::to_string_pretty(self).expect("serializes"));
    }

    /// Puts a document at the top of Open Recent.
    pub fn add_recent(&mut self, doc: &Path) {
        // Absolute, but not canonicalized: on Windows that gives \\?\ paths.
        let doc = std::path::absolute(doc).unwrap_or_else(|_| doc.to_path_buf());
        self.recent.retain(|p| p != &doc);
        self.recent.insert(0, doc);
        self.recent.truncate(RECENT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_documents_are_newest_first_without_duplicates() {
        let mut s = State::default();
        let doc = |name: &str| std::env::temp_dir().join(name);
        for p in ["a", "b", "a", "c"] {
            s.add_recent(&doc(p));
        }
        let names: Vec<&str> = s.recent.iter().map(|p| p.file_name().unwrap().to_str().unwrap()).collect();
        assert_eq!(names, ["c", "a", "b"]);
        assert!(!s.recent[0].to_string_lossy().starts_with(r"\\?\"), "plain paths, not verbatim ones");
        for i in 0..20 {
            s.add_recent(Path::new(&format!("/doc{i}")));
        }
        assert_eq!(s.recent.len(), RECENT);
    }
}
