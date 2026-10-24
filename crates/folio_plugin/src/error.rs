//! Errors from loading and running plugins.

use std::path::PathBuf;
use std::time::Duration;

/// Everything that can go wrong with a plugin. None of these stop the
/// editor; they're reported and the plugin is skipped or restarted.
#[derive(Debug, Clone, thiserror::Error)]
pub enum PluginError {
    /// `plugin.toml` is missing something, has a typo, or asks for a
    /// capability in a way Folio doesn't understand.
    #[error("{}: {message}", path.display())]
    Manifest { path: PathBuf, message: String },

    /// The `.wasm` file couldn't be read, isn't a component, or imports
    /// something Folio doesn't provide.
    #[error("couldn't load plugin \"{plugin}\": {message}")]
    Load { plugin: String, message: String },

    /// The plugin ran longer than the time it gets per event, and was stopped.
    #[error("plugin \"{plugin}\" took longer than {limit:?} and was stopped")]
    Timeout { plugin: String, limit: Duration },

    /// The plugin crashed (a WebAssembly trap, or it ran out of memory).
    #[error("plugin \"{plugin}\" crashed: {message}")]
    Trap { plugin: String, message: String },

    /// The plugin handled the event and reported an error itself.
    #[error("plugin \"{plugin}\": {message}")]
    Guest { plugin: String, message: String },

    /// The plugin failed too many times in a row and was turned off until
    /// plugins are reloaded.
    #[error("plugin \"{plugin}\" was turned off after failing {failures} times in a row")]
    Disabled { plugin: String, failures: u32 },

    /// No loaded plugin has that name.
    #[error("no plugin named \"{0}\" is loaded")]
    NotLoaded(String),

    /// The runtime's thread couldn't start, or has already stopped.
    #[error("the plugin runtime isn't running: {0}")]
    Runtime(String),
}
