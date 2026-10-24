//! The execution API the editor uses: send events, collect results, never
//! wait.
//!
//! Plugins run on one background thread with its own Tokio runtime. Each
//! plugin is a task with its own queue, so a slow plugin only delays
//! itself. A separate ticker thread advances Wasmtime's epoch every 10 ms;
//! at each tick a running plugin yields, which keeps plugins fair and lets
//! one that runs past its timeout be stopped even if it never returns.
//!
//! ```text
//!  editor thread                      plugin thread
//!  ─────────────                      ─────────────
//!  set_document(snapshot)
//!  dispatch(Event::Changed) ──queue──▶ router ──▶ plugin A task ─┐
//!                                             └─▶ plugin B task ─┤
//!  poll() ◀────────────────outputs (edits, notices, errors)───────┘
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use tokio::sync::mpsc as async_mpsc;

use crate::document::{DocumentSnapshot, Event, PluginOutput};
use crate::host::{PluginPre, PluginState, WitEvent};
use crate::manager::{self, Instance, Loader, PluginInfo, PluginPackage};
use crate::sandbox::Limits;
use crate::PluginError;

/// How often running plugins yield (and timeouts are checked).
const TICK: Duration = Duration::from_millis(10);

/// Called whenever plugins have produced output, so the editor can wake up
/// and [`poll`](PluginRuntime::poll) (for egui: `ctx.request_repaint()`).
pub type Waker = Arc<dyn Fn() + Send + Sync>;

pub struct RuntimeConfig {
    /// Where plugins are installed. Defaults to `~/.folio/plugins`.
    pub plugins_dir: PathBuf,
    pub limits: Limits,
    pub waker: Option<Waker>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            plugins_dir: manager::default_plugins_dir(),
            limits: Limits::default(),
            waker: None,
        }
    }
}

enum Control {
    Dispatch {
        /// None sends to every plugin that subscribed to the event.
        to: Option<String>,
        event: Event,
        snapshot: Option<Arc<DocumentSnapshot>>,
    },
    Shutdown,
}

type Queued = (Event, Option<Arc<DocumentSnapshot>>);

/// Runs plugins in the background. Create one when the editor starts; drop
/// it (or call [`shutdown`](Self::shutdown)) when the editor quits.
///
/// Every method returns immediately. Results arrive through
/// [`poll`](Self::poll).
pub struct PluginRuntime {
    control: async_mpsc::UnboundedSender<Control>,
    outputs: mpsc::Receiver<PluginOutput>,
    snapshot: Option<Arc<DocumentSnapshot>>,
    plugins: Vec<PluginInfo>,
    thread: Option<JoinHandle<()>>,
    ticker: Option<(Arc<AtomicBool>, JoinHandle<()>)>,
}

impl PluginRuntime {
    /// Starts the runtime. Plugins are found and compiled in the
    /// background; each reports [`PluginOutput::Loaded`] or
    /// [`PluginOutput::Error`] when it's ready, then gets
    /// [`Event::Startup`].
    pub fn start(config: RuntimeConfig) -> Result<Self, PluginError> {
        let loader = Loader::new()?;
        let (control, control_rx) = async_mpsc::unbounded_channel();
        let (outputs_tx, outputs) = mpsc::channel();

        let stop = Arc::new(AtomicBool::new(false));
        let ticker = {
            let (engine, stop) = (loader.engine.clone(), stop.clone());
            std::thread::Builder::new()
                .name("folio-plugin-ticker".into())
                .spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        std::thread::sleep(TICK);
                        engine.increment_epoch();
                    }
                })
                .map_err(|e| PluginError::Runtime(e.to_string()))?
        };

        let thread = std::thread::Builder::new()
            .name("folio-plugins".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build();
                match rt {
                    Ok(rt) => rt.block_on(router(loader, config, control_rx, outputs_tx)),
                    Err(e) => {
                        let _ = outputs_tx.send(PluginOutput::Error(PluginError::Runtime(e.to_string())));
                    }
                }
            })
            .map_err(|e| PluginError::Runtime(e.to_string()))?;

        Ok(Self {
            control,
            outputs,
            snapshot: None,
            plugins: Vec::new(),
            thread: Some(thread),
            ticker: Some((stop, ticker)),
        })
    }

    /// Publishes the document as it is now. Events sent after this carry it.
    /// Call it when the document or selection changes; it's cheap.
    pub fn set_document(&mut self, snapshot: DocumentSnapshot) {
        self.snapshot = Some(Arc::new(snapshot));
    }

    /// Sends an event to every plugin that subscribed to it.
    pub fn dispatch(&self, event: Event) {
        self.send(None, event);
    }

    /// Runs one of a plugin's commands (its `id` from `plugin.toml`).
    pub fn run_command(&self, plugin: &str, command: &str) {
        self.send(Some(plugin.to_owned()), Event::Command(command.to_owned()));
    }

    fn send(&self, to: Option<String>, event: Event) {
        let _ = self.control.send(Control::Dispatch {
            to,
            event,
            snapshot: self.snapshot.clone(),
        });
    }

    /// Everything plugins have produced since the last call. Never waits.
    pub fn poll(&mut self) -> Vec<PluginOutput> {
        let out: Vec<PluginOutput> = self.outputs.try_iter().collect();
        for o in &out {
            if let PluginOutput::Loaded(info) = o {
                self.plugins.push(info.clone());
            }
        }
        out
    }

    /// Plugins that have loaded so far (as reported through [`poll`](Self::poll)).
    pub fn plugins(&self) -> &[PluginInfo] {
        &self.plugins
    }

    /// Stops every plugin and waits for the background threads. Plugins in
    /// the middle of an event get at most their timeout to finish it.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        let _ = self.control.send(Control::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if let Some((stop, ticker)) = self.ticker.take() {
            stop.store(true, Ordering::Relaxed);
            let _ = ticker.join();
        }
    }
}

impl Drop for PluginRuntime {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Loads every plugin, then forwards events to them until shutdown.
async fn router(
    loader: Loader,
    config: RuntimeConfig,
    mut control: async_mpsc::UnboundedReceiver<Control>,
    outputs: mpsc::Sender<PluginOutput>,
) {
    let wake = {
        let waker = config.waker.clone();
        move || {
            if let Some(w) = &waker {
                w();
            }
        }
    };
    let (packages, errors) = manager::discover(&config.plugins_dir);
    for e in errors {
        let _ = outputs.send(PluginOutput::Error(e));
    }

    let mut queues: HashMap<String, (async_mpsc::UnboundedSender<Queued>, Arc<PluginPackage>)> = HashMap::new();
    let mut tasks = tokio::task::JoinSet::new();
    for package in packages {
        // Compiling is CPU work; keep it off the thread plugins run on.
        let compiled = {
            let (loader, package) = (loader.clone(), package.clone());
            tokio::task::spawn_blocking(move || loader.compile(&package)).await
        };
        let pre = match compiled {
            Ok(Ok(pre)) => pre,
            Ok(Err(e)) => {
                let _ = outputs.send(PluginOutput::Error(e));
                continue;
            }
            Err(e) => {
                let _ = outputs.send(PluginOutput::Error(PluginError::Runtime(e.to_string())));
                continue;
            }
        };
        let package = Arc::new(package);
        let (tx, rx) = async_mpsc::unbounded_channel();
        let _ = tx.send((Event::Startup, None));
        queues.insert(package.manifest.name.clone(), (tx, package.clone()));
        let _ = outputs.send(PluginOutput::Loaded(package.info()));
        tasks.spawn(run_plugin(
            loader.clone(),
            pre,
            package,
            rx,
            outputs.clone(),
            config.limits.clone(),
            config.waker.clone(),
        ));
    }
    wake();

    while let Some(message) = control.recv().await {
        match message {
            Control::Dispatch { to: Some(name), event, snapshot } => match queues.get(&name) {
                Some((tx, _)) => {
                    let _ = tx.send((event, snapshot));
                }
                None => {
                    let _ = outputs.send(PluginOutput::Error(PluginError::NotLoaded(name)));
                    wake();
                }
            },
            Control::Dispatch { to: None, event, snapshot } => {
                for (tx, package) in queues.values() {
                    if package.manifest.wants(&event) {
                        let _ = tx.send((event.clone(), snapshot.clone()));
                    }
                }
            }
            Control::Shutdown => break,
        }
    }
    // Closing the queues ends each plugin's task after its current event.
    queues.clear();
    while tasks.join_next().await.is_some() {}
}

/// One plugin's task: handles its events in order, restarting the plugin
/// after a crash, and turning it off if it keeps failing.
async fn run_plugin(
    loader: Loader,
    pre: PluginPre<PluginState>,
    package: Arc<PluginPackage>,
    mut queue: async_mpsc::UnboundedReceiver<Queued>,
    outputs: mpsc::Sender<PluginOutput>,
    limits: Limits,
    waker: Option<Waker>,
) {
    let name = package.manifest.name.clone();
    let report = |e: PluginError| {
        let _ = outputs.send(PluginOutput::Error(e));
    };
    let mut instance: Option<Instance> = None;
    let mut failures = 0u32;

    while let Some(first) = queue.recv().await {
        let mut batch = vec![first];
        while let Ok(more) = queue.try_recv() {
            batch.push(more);
        }
        for (event, snapshot) in coalesce(batch) {
            if instance.is_none() {
                match manager::instantiate(&loader, &pre, &package, outputs.clone(), &limits).await {
                    Ok(i) => instance = Some(i),
                    Err(e) => {
                        report(e);
                        failures += 1;
                    }
                }
            }
            if let Some(i) = &mut instance {
                match handle(i, &event, snapshot).await {
                    Ok(()) => failures = 0,
                    Err(e @ PluginError::Guest { .. }) => report(e),
                    Err(e) => {
                        // After a trap the instance's state can't be trusted.
                        report(e);
                        instance = None;
                        failures += 1;
                    }
                }
            }
            if let Some(w) = &waker {
                w();
            }
            if failures >= limits.max_failures {
                report(PluginError::Disabled { plugin: name.clone(), failures });
                if let Some(w) = &waker {
                    w();
                }
                return;
            }
        }
    }
}

/// Runs one event in a plugin, within its timeout and budget.
async fn handle(instance: &mut Instance, event: &Event, snapshot: Option<Arc<DocumentSnapshot>>) -> Result<(), PluginError> {
    let Instance { store, bindings } = instance;
    store.data_mut().begin_event(snapshot);
    store.set_epoch_deadline(1);
    let result = bindings.folio_plugin_guest().call_on_event(&mut *store, &WitEvent::from(event)).await;
    let state = store.data_mut();
    state.snapshot = None;
    let plugin = state.plugin.clone();
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(message)) => Err(PluginError::Guest { plugin, message }),
        Err(_) if state.timed_out() => Err(PluginError::Timeout {
            plugin,
            limit: state.timeout(),
        }),
        Err(trap) => Err(PluginError::Trap {
            plugin,
            message: format!("{trap:#}"),
        }),
    }
}

/// Drops `changed` and `selection-changed` events that a later one of the
/// same kind makes redundant, so a plugin that falls behind catches up
/// with the newest document instead of replaying every keystroke.
fn coalesce(batch: Vec<Queued>) -> Vec<Queued> {
    let mut keep = vec![true; batch.len()];
    for (i, (event, _)) in batch.iter().enumerate() {
        if event.coalesces() && batch[i + 1..].iter().any(|(later, _)| later == event) {
            keep[i] = false;
        }
    }
    batch.into_iter().zip(keep).filter_map(|(q, k)| k.then_some(q)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backlog_of_changes_collapses_to_the_newest() {
        let snap = |r| Some(Arc::new(DocumentSnapshot { revision: r, ..Default::default() }));
        let batch = vec![
            (Event::Changed, snap(1)),
            (Event::SelectionChanged, snap(1)),
            (Event::Command("a".into()), snap(2)),
            (Event::Changed, snap(3)),
            (Event::Saved("/x".into()), snap(3)),
            (Event::Changed, snap(4)),
        ];
        let kept: Vec<(String, u64)> = coalesce(batch)
            .into_iter()
            .map(|(e, s)| (e.kind().to_owned(), s.unwrap().revision))
            .collect();
        assert_eq!(
            kept,
            [("selection-changed".into(), 1), ("command".into(), 2), ("saved".into(), 3), ("changed".into(), 4)]
        );
    }
}
