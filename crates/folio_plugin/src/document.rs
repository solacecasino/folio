//! What the editor shares with plugins, and what plugins hand back.
//!
//! These are plain Rust types, independent of WebAssembly, so the editor
//! never touches the generated bindings. The editor publishes a
//! [`DocumentSnapshot`] whenever the document or selection changes; every
//! event carries the snapshot current when it was sent, so a plugin sees a
//! consistent document even while the person keeps typing.

use std::path::PathBuf;

/// A place in the document: a paragraph index and a character index
/// within that paragraph (characters, not bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Position {
    pub paragraph: u32,
    pub character: u32,
}

/// The selected text and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// Start and end in document order.
    pub start: Position,
    pub end: Position,
    /// Paragraphs separated by `\n`.
    pub text: String,
}

/// The document as plugins see it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DocumentSnapshot {
    /// Increases with every change. Edits carry the revision they were made
    /// against, so the editor can tell when one is stale.
    pub revision: u64,
    pub title: String,
    pub path: Option<PathBuf>,
    pub theme: String,
    pub paragraphs: u32,
    pub words: u32,
    pub characters: u32,
    /// None when there's only a caret.
    pub selection: Option<Selection>,
}

/// Something that happened in the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Sent once, right after a plugin loads.
    Startup,
    /// One of the plugin's commands was chosen (the `id` from its manifest).
    /// Sent only to that plugin.
    Command(String),
    /// The document changed.
    Changed,
    /// The document was saved.
    Saved(PathBuf),
    /// The selection or caret moved.
    SelectionChanged,
}

impl Event {
    /// The name plugins subscribe with in `plugin.toml` (`events = [...]`).
    pub fn kind(&self) -> &'static str {
        match self {
            Event::Startup => "startup",
            Event::Command(_) => "command",
            Event::Changed => "changed",
            Event::Saved(_) => "saved",
            Event::SelectionChanged => "selection-changed",
        }
    }

    /// Events every plugin gets without subscribing.
    pub(crate) fn always_delivered(&self) -> bool {
        matches!(self, Event::Startup | Event::Command(_))
    }

    /// Events where only the latest one matters, so a backlog of them can
    /// be collapsed into one when a plugin falls behind.
    pub(crate) fn coalesces(&self) -> bool {
        matches!(self, Event::Changed | Event::SelectionChanged)
    }

    pub const SUBSCRIBABLE: [&'static str; 3] = ["changed", "saved", "selection-changed"];
}

/// A change a plugin asked for. The editor applies it on its own thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// Insert at the caret, replacing any selection; `\n` starts a paragraph.
    InsertText(String),
}

/// How serious a log line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// What plugins send back to the editor. Collect these each frame with
/// [`PluginRuntime::poll`](crate::PluginRuntime::poll).
#[derive(Debug, Clone)]
pub enum PluginOutput {
    /// A plugin finished loading.
    Loaded(crate::PluginInfo),
    /// A plugin couldn't be loaded, or a plugin failed while handling an event.
    Error(crate::PluginError),
    /// Apply this edit if `revision` still matches the document (or rebase it).
    Edit { plugin: String, revision: u64, edit: Edit },
    /// Show this message briefly.
    Notify { plugin: String, message: String },
    Log { plugin: String, level: LogLevel, message: String },
}
