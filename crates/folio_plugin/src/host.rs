//! The host side of the WIT interface: what plugins can call, with every
//! call checked against the plugin's capabilities and per-event budget.

use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use wasmtime::StoreLimits;
use wasmtime::component::ResourceTable;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::document::{DocumentSnapshot, Edit, LogLevel, PluginOutput};
use crate::manifest::Capabilities;
use crate::sandbox::Limits;

wasmtime::component::bindgen!({
    path: "wit",
    world: "plugin",
    // Calls into the plugin are async so a slow plugin yields to others
    // (and to the timeout) instead of holding the thread.
    exports: { default: async },
});

pub(crate) use exports::folio::plugin::guest::Event as WitEvent;
use folio::plugin::document::{self as wit_doc, Denied};
use folio::plugin::host::{self as wit_host, Level};

/// Everything a plugin's store holds.
pub(crate) struct PluginState {
    pub plugin: String,
    capabilities: Capabilities,
    /// The document as of the event being handled; None between events.
    pub snapshot: Option<Arc<DocumentSnapshot>>,
    outputs: Sender<PluginOutput>,
    limits: Limits,
    /// What's left of this event's budget.
    messages_left: u32,
    bytes_left: usize,
    /// When the current event started, for the timeout.
    pub started: Instant,
    pub store_limits: StoreLimits,
    wasi: WasiCtx,
    table: ResourceTable,
}

impl PluginState {
    pub fn new(plugin: String, capabilities: Capabilities, wasi: WasiCtx, outputs: Sender<PluginOutput>, limits: Limits) -> Self {
        Self {
            plugin,
            capabilities,
            snapshot: None,
            outputs,
            store_limits: limits.store_limits(),
            messages_left: limits.messages_per_event,
            bytes_left: limits.bytes_per_event,
            limits,
            started: Instant::now(),
            wasi,
            table: ResourceTable::new(),
        }
    }

    /// Resets the budget and clock for a new event.
    pub fn begin_event(&mut self, snapshot: Option<Arc<DocumentSnapshot>>) {
        self.snapshot = snapshot;
        self.messages_left = self.limits.messages_per_event;
        self.bytes_left = self.limits.bytes_per_event;
        self.started = Instant::now();
    }

    pub fn timed_out(&self) -> bool {
        self.started.elapsed() > self.limits.event_timeout
    }

    pub fn timeout(&self) -> Duration {
        self.limits.event_timeout
    }

    /// Spends budget for one message carrying `bytes` of text.
    fn spend(&mut self, bytes: usize) -> bool {
        if self.messages_left == 0 || self.bytes_left < bytes {
            return false;
        }
        self.messages_left -= 1;
        self.bytes_left -= bytes;
        true
    }

    fn send(&self, output: PluginOutput) {
        // The editor dropping its receiver just means nobody is listening.
        let _ = self.outputs.send(output);
    }

    fn readable(&self) -> Result<&DocumentSnapshot, Denied> {
        if !self.capabilities.can_read_document() {
            return Err(Denied::MissingCapability);
        }
        self.snapshot.as_deref().ok_or(Denied::NoDocument)
    }
}

impl WasiView for PluginState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

fn position(p: crate::Position) -> wit_doc::Position {
    wit_doc::Position {
        paragraph: p.paragraph,
        character: p.character,
    }
}

impl wit_doc::Host for PluginState {
    fn get_selection(&mut self) -> Result<Option<wit_doc::Selection>, Denied> {
        let doc = self.readable()?;
        Ok(doc.selection.as_ref().map(|s| wit_doc::Selection {
            start: position(s.start),
            end: position(s.end),
            text: s.text.clone(),
        }))
    }

    fn get_metadata(&mut self) -> Result<wit_doc::Metadata, Denied> {
        let doc = self.readable()?;
        Ok(wit_doc::Metadata {
            title: doc.title.clone(),
            path: doc.path.as_ref().map(|p| p.display().to_string()),
            theme: doc.theme.clone(),
            paragraphs: doc.paragraphs,
            words: doc.words,
            characters: doc.characters,
            revision: doc.revision,
        })
    }

    fn insert_text(&mut self, text: String) -> Result<(), Denied> {
        if !self.capabilities.can_edit_document() {
            return Err(Denied::MissingCapability);
        }
        let revision = self.snapshot.as_ref().ok_or(Denied::NoDocument)?.revision;
        if !self.spend(text.len()) {
            return Err(Denied::LimitReached);
        }
        self.send(PluginOutput::Edit {
            plugin: self.plugin.clone(),
            revision,
            edit: Edit::InsertText(text),
        });
        Ok(())
    }
}

impl wit_host::Host for PluginState {
    fn log(&mut self, level: Level, message: String) {
        if self.spend(message.len()) {
            let level = match level {
                Level::Debug => LogLevel::Debug,
                Level::Info => LogLevel::Info,
                Level::Warn => LogLevel::Warn,
                Level::Error => LogLevel::Error,
            };
            self.send(PluginOutput::Log {
                plugin: self.plugin.clone(),
                level,
                message,
            });
        }
    }

    fn notify(&mut self, message: String) {
        if self.spend(message.len()) {
            self.send(PluginOutput::Notify {
                plugin: self.plugin.clone(),
                message,
            });
        }
    }
}

impl From<&crate::Event> for WitEvent {
    fn from(e: &crate::Event) -> Self {
        match e {
            crate::Event::Startup => WitEvent::Startup,
            crate::Event::Command(id) => WitEvent::Command(id.clone()),
            crate::Event::Changed => WitEvent::Changed,
            crate::Event::Saved(p) => WitEvent::Saved(p.display().to_string()),
            crate::Event::SelectionChanged => WitEvent::SelectionChanged,
        }
    }
}
