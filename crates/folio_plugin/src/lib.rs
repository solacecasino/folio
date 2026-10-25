//! # folio_plugin
//!
//! Folio's embedded plugin runtime: plugins are WebAssembly components
//! (WASI Preview 2) run by [Wasmtime](https://wasmtime.dev) inside Folio's
//! process, each in its own sandbox.
//!
//! - **The interface** is `wit/folio-plugin.wit`. Plugins can read the
//!   selection and document metadata, insert text, log and show messages,
//!   and handle events (`startup`, their own commands, `changed`, `saved`,
//!   `selection-changed`).
//! - **Sandboxing is capability-based and denies by default.** A plugin gets
//!   no filesystem, no network, no environment and no document access unless
//!   its `plugin.toml` asks (see [`manifest`]). Document access is checked
//!   on every call; filesystem and network grants become a WASI context that
//!   can reach only what was granted (see [`sandbox`]).
//! - **Limits apply to everyone:** a time limit per event (enforced with
//!   epoch interruption, so even an infinite loop is stopped), a memory
//!   limit, and a budget of edits and messages per event ([`Limits`]). A
//!   plugin that crashes is restarted; one that keeps failing is turned off.
//! - **The editor never waits.** [`PluginRuntime`] runs plugins on a
//!   background thread. The editor publishes a [`DocumentSnapshot`],
//!   dispatches [`Event`]s, and each frame collects [`PluginOutput`]s
//!   (edits to apply, messages to show) with [`PluginRuntime::poll`].
//!
//! ```no_run
//! use folio_plugin::{DocumentSnapshot, Event, PluginOutput, PluginRuntime, RuntimeConfig};
//!
//! let mut plugins = PluginRuntime::start(RuntimeConfig::default())?;
//!
//! // Whenever the document or selection changes:
//! plugins.set_document(DocumentSnapshot { revision: 1, title: "Essay".into(), ..Default::default() });
//! plugins.dispatch(Event::Changed);
//!
//! // Once per frame:
//! for output in plugins.poll() {
//!     match output {
//!         PluginOutput::Edit { revision, edit, .. } => { /* apply if `revision` is current */ }
//!         PluginOutput::Notify { message, .. } => { /* show it */ }
//!         other => eprintln!("{other:?}"),
//!     }
//! }
//! # Ok::<(), folio_plugin::PluginError>(())
//! ```
//!
//! Plugins live in `plugins/` in Folio's config folder, one folder each with `plugin.toml`
//! and a `.wasm` component built for `wasm32-wasip2`. See
//! `tests/guest/` for a complete plugin written in Rust.

pub mod document;
mod error;
mod host;
pub mod manager;
pub mod manifest;
mod runtime;
pub mod sandbox;

pub use document::{DocumentSnapshot, Edit, Event, LogLevel, PanelLine, PluginOutput, Position, Selection};
pub use error::PluginError;
pub use manager::{PluginInfo, default_plugins_dir};
pub use manifest::{Capabilities, Manifest};
pub use runtime::{PluginRuntime, RuntimeConfig, Waker};
pub use sandbox::Limits;
