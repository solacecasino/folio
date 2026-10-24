//! Adapters: optional add-ons that run as separate programs, in the spirit
//! of foobar2000's components. Folio stays bare; everything extra is an
//! adapter someone installs on purpose.
//!
//! An adapter is a folder holding `adapter.toml` and a program. Folio
//! starts the program only when it's needed and talks to it with one JSON
//! message per line over stdin/stdout. See `docs/adapters.md` for the
//! protocol.
//!
//! Nothing is installed by default. People add adapters from the Adapters
//! window (a downloaded `.zip` or folder), and Folio keeps them in the
//! `adapters` folder of its config folder. Adapters used to be called
//! components; a `components/` folder there and `component.toml` still work.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use serde::Deserialize;

use crate::doc::Pos;
use serde_json::{Value, json};

pub const PROTOCOL_VERSION: u32 = 3;

pub const READ: &str = "document.read";
pub const EDIT: &str = "document.edit";

#[derive(Clone, Debug, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub description: String,
    /// Program and arguments, run from the adapter's folder.
    pub run: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub start: Start,
    /// Events to receive: "changed", "saved", "opened".
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub commands: Vec<CommandSpec>,
    /// Panels this adapter can fill; users place them in settings.toml.
    #[serde(default)]
    pub panels: Vec<PanelDecl>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PanelDecl {
    pub id: String,
    pub title: String,
}

/// Someone else's caret and selection, shown by a sharing adapter.
#[derive(Clone, Debug, PartialEq)]
pub struct Peer {
    pub name: String,
    pub color: String,
    pub cursor: Pos,
    pub anchor: Pos,
}

/// One line of an adapter panel. Lines with a command are clickable.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelLine {
    pub text: String,
    pub command: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Start {
    /// Started the first time one of its commands is used.
    #[default]
    Command,
    /// Started when Folio opens.
    Launch,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CommandSpec {
    pub id: String,
    pub label: String,
    /// Default shortcut, e.g. "cmd+shift+d" (users can remap it in keys.toml).
    pub keys: Option<String>,
    /// Also show the command in Folio's "file", "edit", "format" or "view"
    /// menu. Every command is in the Adapters window either way.
    pub menu: Option<String>,
}

/// A message from an adapter that the app has to act on.
#[derive(Debug)]
pub struct Call {
    pub adapter: usize,
    /// Present for requests, which need a reply.
    pub id: Option<Value>,
    pub method: String,
    pub params: Value,
}

enum Incoming {
    Line(String),
    Exited,
}

struct Running {
    child: Child,
    stdin: ChildStdin,
    generation: u64,
}

pub struct Adapter {
    pub manifest: Manifest,
    dir: PathBuf,
    running: Option<Running>,
    generation: u64,
    /// Text the adapter shows in the status line.
    pub status: Option<String>,
    /// Panel contents by panel id.
    pub panels: BTreeMap<String, Vec<PanelLine>>,
    /// Set when the user said no or it failed to start, so Folio doesn't
    /// keep asking every frame a panel wants it.
    blocked: bool,
    /// Uninstalled or replaced while Folio was open. Kept as a stopped
    /// placeholder so every other adapter keeps its index.
    pub removed: bool,
}

impl Adapter {
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// The user said no to it (or it failed to start) this session.
    pub fn is_blocked(&self) -> bool {
        self.blocked
    }
}

pub type Waker = Arc<dyn Fn() + Send + Sync>;
pub type Approver = Box<dyn Fn(&Manifest) -> bool>;

pub struct Host {
    pub all: Vec<Adapter>,
    tx: Sender<(usize, u64, Incoming)>,
    rx: Receiver<(usize, u64, Incoming)>,
    waker: Waker,
    approver: Approver,
    launched: bool,
    /// Set when an adapter starts, so the app can send it the current state.
    pub just_started: bool,
    /// An adapter running a shared session: (adapter, whether it handles undo).
    pub session: Option<(usize, bool)>,
    /// Other people's carets, from the adapter running the session.
    pub peers: Vec<Peer>,
}

impl Host {
    /// `waker` is called from background threads when an adapter sends
    /// something, so an idle app can wake up and handle it.
    pub fn new(adapters: Vec<Adapter>, waker: Waker, approver: Approver) -> Self {
        let (tx, rx) = channel();
        Self {
            all: adapters,
            tx,
            rx,
            waker,
            approver,
            launched: false,
            just_started: false,
            session: None,
            peers: Vec::new(),
        }
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.all.iter().position(|c| !c.removed && c.manifest.name == name)
    }

    /// Installed adapters, with their indexes.
    pub fn installed(&self) -> impl Iterator<Item = (usize, &Adapter)> {
        self.all.iter().enumerate().filter(|(_, a)| !a.removed)
    }

    /// Stops an adapter's program and clears what it showed.
    pub fn stop(&mut self, i: usize) {
        self.send(i, json!({ "method": "shutdown" }));
        let a = &mut self.all[i];
        if let Some(mut r) = a.running.take() {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let _ = r.child.kill();
            let _ = r.child.wait();
        }
        a.status = None;
        a.panels.clear();
        if self.session.is_some_and(|(s, _)| s == i) {
            self.session = None;
            self.peers.clear();
        }
    }

    /// Brings the list in line with what's installed on disk: adapters still
    /// there keep running, new ones are added, gone or replaced ones stop.
    pub fn sync(&mut self, found: Vec<Adapter>) {
        for i in 0..self.all.len() {
            let a = &self.all[i];
            let same = found
                .iter()
                .any(|f| f.dir == a.dir && f.manifest.name == a.manifest.name && f.manifest.version == a.manifest.version);
            if !a.removed && !same {
                self.stop(i);
                self.all[i].removed = true;
            }
        }
        for f in found {
            let known = self.all.iter().any(|a| {
                !a.removed && a.dir == f.dir && a.manifest.name == f.manifest.name && a.manifest.version == f.manifest.version
            });
            if !known {
                self.all.push(f);
            }
        }
    }

    /// Starts an adapter whose panel is on screen. Asks for approval once;
    /// after a "no" it stays stopped until Folio restarts.
    pub fn ensure_running(&mut self, i: usize) -> Result<(), String> {
        if self.all[i].running.is_some() || self.all[i].blocked {
            return Ok(());
        }
        self.start(i).inspect_err(|_| self.all[i].blocked = true)
    }

    pub fn has_permission(&self, i: usize, permission: &str) -> bool {
        self.all[i].manifest.permissions.iter().any(|p| p == permission)
    }

    fn start(&mut self, i: usize) -> Result<(), String> {
        if self.all[i].running.is_some() {
            return Ok(());
        }
        if !(self.approver)(&self.all[i].manifest) {
            return Err(format!("{} wasn't allowed to run", self.all[i].manifest.name));
        }

        let c = &mut self.all[i];
        let mut child = spawn(&c.dir, &c.manifest.run).map_err(|e| format!("Couldn't start {}: {e}", c.manifest.name))?;

        c.generation += 1;
        let generation = c.generation;
        let stdout = child.stdout.take().expect("stdout is piped");
        let stdin = child.stdin.take().expect("stdin is piped");
        let (tx, waker) = (self.tx.clone(), self.waker.clone());
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send((i, generation, Incoming::Line(line))).is_err() {
                    return;
                }
                waker();
            }
            let _ = tx.send((i, generation, Incoming::Exited));
            waker();
        });
        c.running = Some(Running {
            child,
            stdin,
            generation,
        });

        let permissions = c.manifest.permissions.clone();
        self.just_started = true;
        self.send(
            i,
            json!({
                "method": "initialize",
                "params": {
                    "folio": env!("CARGO_PKG_VERSION"),
                    "protocol": PROTOCOL_VERSION,
                    "permissions": permissions,
                },
            }),
        );
        Ok(())
    }

    fn send(&mut self, i: usize, message: Value) {
        if let Some(r) = &mut self.all[i].running {
            let line = format!("{message}\n");
            // An adapter that has quit just misses the message.
            let _ = r.stdin.write_all(line.as_bytes()).and_then(|()| r.stdin.flush());
        }
    }

    /// Runs an adapter's command by name and id, starting it if needed.
    pub fn run_command(&mut self, name: &str, id: &str) -> Result<(), String> {
        let i = self
            .index_of(name)
            .ok_or_else(|| format!("no adapter named \"{name}\" is installed"))?;
        self.all[i].blocked = false;
        self.start(i)?;
        self.send(i, json!({ "method": "command", "params": { "command": id } }));
        Ok(())
    }

    /// Sends an event to every running adapter that asked for it.
    /// Whether any running adapter asked for an event.
    pub fn wants(&self, event: &str) -> bool {
        self.all
            .iter()
            .any(|c| c.running.is_some() && c.manifest.events.iter().any(|e| e == event))
    }

    /// Sends a notification with no params, e.g. `undo` during a session.
    pub fn notify(&mut self, i: usize, method: &str) {
        self.send(i, json!({ "method": method }));
    }

    pub fn emit(&mut self, event: &str, params: Value) {
        // Snapshots carry the document's text, so they need read permission.
        let needs_read = event == "snapshot";
        for i in 0..self.all.len() {
            let c = &self.all[i];
            if needs_read && !self.has_permission(i, READ) {
                continue;
            }
            if c.running.is_some() && c.manifest.events.iter().any(|e| e == event) {
                let mut params = params.clone();
                params["kind"] = json!(event);
                self.send(i, json!({ "method": "event", "params": params }));
            }
        }
    }

    pub fn reply(&mut self, i: usize, id: Value, result: Result<Value, String>) {
        let message = match result {
            Ok(result) => json!({ "id": id, "result": result }),
            Err(error) => json!({ "id": id, "error": error }),
        };
        self.send(i, message);
    }

    /// Starts `start = "launch"` adapters (once), handles messages Folio can
    /// answer on its own, and returns the rest for the app.
    pub fn poll(&mut self) -> (Vec<Call>, Vec<String>) {
        let mut errors = Vec::new();
        if !self.launched {
            self.launched = true;
            for i in 0..self.all.len() {
                if self.all[i].manifest.start == Start::Launch
                    && let Err(e) = self.start(i)
                {
                    errors.push(e);
                }
            }
        }

        let mut calls = Vec::new();
        while let Ok((i, generation, incoming)) = self.rx.try_recv() {
            // Ignore leftovers from an earlier run of the same adapter.
            if self.all[i].running.as_ref().map(|r| r.generation) != Some(generation) {
                continue;
            }
            match incoming {
                Incoming::Exited => {
                    let c = &mut self.all[i];
                    if let Some(mut r) = c.running.take() {
                        let _ = r.child.wait();
                    }
                    c.status = None;
                    c.panels.clear();
                    if self.session.is_some_and(|(s, _)| s == i) {
                        self.session = None;
                        self.peers.clear();
                    }
                }
                Incoming::Line(line) => {
                    let message: Value = match serde_json::from_str(&line) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("folio: {} sent invalid JSON: {e}", self.all[i].manifest.name);
                            continue;
                        }
                    };
                    let id = message.get("id").cloned();
                    let Some(method) = message.get("method").and_then(Value::as_str).map(str::to_owned)
                    else {
                        continue; // Folio never sends requests, so there are no replies to read.
                    };
                    let params = message.get("params").cloned().unwrap_or(Value::Null);
                    match method.as_str() {
                        "status.set" => {
                            let c = &mut self.all[i];
                            c.status = params.get("text").and_then(Value::as_str).map(str::to_owned);
                            if let Some(id) = id {
                                self.reply(i, id, Ok(Value::Null));
                            }
                        }
                        "session.set" => {
                            let active = params.get("active").and_then(Value::as_bool).unwrap_or(false);
                            let undo = params.get("handles_undo").and_then(Value::as_bool).unwrap_or(false);
                            self.session = active.then_some((i, undo));
                            if !active {
                                self.peers.clear();
                            }
                            if let Some(id) = id {
                                self.reply(i, id, Ok(Value::Null));
                            }
                        }
                        "presence.set" => {
                            self.peers = params
                                .get("peers")
                                .and_then(Value::as_array)
                                .map(|peers| peers.iter().filter_map(peer).collect())
                                .unwrap_or_default();
                            if let Some(id) = id {
                                self.reply(i, id, Ok(Value::Null));
                            }
                        }
                        "panel.set" => {
                            let panel = params.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
                            let lines = params
                                .get("lines")
                                .and_then(Value::as_array)
                                .map(|lines| lines.iter().map(panel_line).collect())
                                .unwrap_or_default();
                            self.all[i].panels.insert(panel, lines);
                            if let Some(id) = id {
                                self.reply(i, id, Ok(Value::Null));
                            }
                        }
                        "log" => eprintln!(
                            "[{}] {}",
                            self.all[i].manifest.name,
                            params.get("message").and_then(Value::as_str).unwrap_or_default()
                        ),
                        _ => calls.push(Call {
                            adapter: i,
                            id,
                            method,
                            params,
                        }),
                    }
                }
            }
        }
        (calls, errors)
    }

    pub fn status_items(&self) -> impl Iterator<Item = &str> {
        self.all.iter().filter(|c| !c.removed).filter_map(|c| c.status.as_deref())
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        for i in 0..self.all.len() {
            self.send(i, json!({ "method": "shutdown" }));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        for c in &mut self.all {
            if let Some(mut r) = c.running.take() {
                let _ = r.child.kill();
                let _ = r.child.wait();
            }
        }
    }
}

/// `{para, ch}` as a document position.
pub fn pos(v: &Value) -> Option<Pos> {
    Some(Pos::new(v.get("para")?.as_u64()? as usize, v.get("ch")?.as_u64()? as usize))
}

fn peer(v: &Value) -> Option<Peer> {
    let cursor = pos(v.get("cursor")?)?;
    Some(Peer {
        name: v.get("name").and_then(Value::as_str).unwrap_or("Someone").to_owned(),
        color: v.get("color").and_then(Value::as_str).unwrap_or("#3d7eff").to_owned(),
        anchor: v.get("anchor").and_then(pos).unwrap_or(cursor),
        cursor,
    })
}

/// A panel line is either a string or `{ "text": ..., "command": ... }`.
fn panel_line(v: &Value) -> PanelLine {
    match v {
        Value::String(text) => PanelLine {
            text: text.clone(),
            command: None,
        },
        _ => PanelLine {
            text: v.get("text").and_then(Value::as_str).unwrap_or_default().to_owned(),
            command: v.get("command").and_then(Value::as_str).map(str::to_owned),
        },
    }
}

/// Starts an adapter's `run` command in its folder. A program given as a
/// path (`./pdf-export`) is found in the adapter's folder, with `.exe` added
/// on Windows; on Windows `python3` also tries the `py` launcher and
/// `python`, which is how Python usually installs there.
fn spawn(dir: &Path, run: &[String]) -> Result<Child, String> {
    let (program, args) = run.split_first().ok_or("adapter.toml has an empty `run`")?;
    let mut tries: Vec<(std::ffi::OsString, Vec<&str>)> = Vec::new();
    if program.contains(['/', '\\']) {
        let path = dir.join(program);
        let exe = path.with_extension("exe");
        let path = if cfg!(windows) && !path.is_file() && exe.is_file() { exe } else { path };
        tries.push((path.into_os_string(), Vec::new()));
    } else {
        if cfg!(windows) && program == "python3" {
            tries.push(("py".into(), vec!["-3"]));
            tries.push(("python".into(), Vec::new()));
        }
        tries.push((program.into(), Vec::new()));
    }

    let mut last = String::new();
    for (program, prefix) in tries {
        let mut command = Command::new(&program);
        command
            .args(prefix)
            .args(args)
            .current_dir(dir)
            // So adapters keep their own files next to Folio's.
            .env("FOLIO_CONFIG_DIR", crate::settings::config_dir())
            // Messages are UTF-8; Python on Windows would otherwise use the code page.
            .env("PYTHONIOENCODING", "utf-8")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        crate::platform::hide_console(&mut command);
        match command.spawn() {
            Ok(child) => return Ok(child),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => last = format!("{} not found", program.to_string_lossy()),
            Err(e) => return Err(e.to_string()),
        }
    }
    Err(last)
}

// ---- Discovery --------------------------------------------------------------

pub fn adapters_dir() -> PathBuf {
    crate::settings::config_dir().join("adapters")
}

/// Installed adapters. Folio only looks in the config folder, so nothing
/// appears unless someone installed it.
pub fn discover() -> Vec<Adapter> {
    discover_in(&[crate::settings::config_dir().join("components"), adapters_dir()])
}

/// Finds adapters in `dirs`; a later folder's adapter replaces an earlier
/// one with the same name.
fn discover_in(dirs: &[PathBuf]) -> Vec<Adapter> {
    let mut found: Vec<Adapter> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        let mut folders: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        folders.sort();
        for folder in folders {
            match load_manifest(&folder) {
                // An installed adapter replaces a bundled one with the same name.
                Ok(manifest) => {
                    found.retain(|c| c.manifest.name != manifest.name);
                    found.push(Adapter {
                        manifest,
                        dir: folder,
                        running: None,
                        generation: 0,
                        status: None,
                        panels: BTreeMap::new(),
                        blocked: false,
                        removed: false,
                    });
                }
                Err(e) => eprintln!("folio: skipping adapter {}: {e}", folder.display()),
            }
        }
    }
    found
}

fn load_manifest(folder: &Path) -> Result<Manifest, String> {
    let manifest = ["adapter.toml", "component.toml"]
        .iter()
        .map(|name| folder.join(name))
        .find(|p| p.exists())
        .ok_or("no adapter.toml")?;
    let src = std::fs::read_to_string(manifest).map_err(|e| e.to_string())?;
    toml::from_str(&src).map_err(|e| e.to_string())
}

// ---- Installing -------------------------------------------------------------

/// An adapter unpacked and checked, ready to install.
pub struct Staged {
    /// The folder holding `adapter.toml`.
    pub dir: PathBuf,
    pub manifest: Manifest,
    /// A temporary folder to delete afterwards (for zips).
    temp: Option<PathBuf>,
}

impl Drop for Staged {
    fn drop(&mut self) {
        if let Some(t) = &self.temp {
            let _ = std::fs::remove_dir_all(t);
        }
    }
}

/// Unpacks a downloaded adapter (`.zip` or folder) and reads its manifest,
/// without installing anything yet.
pub fn stage(source: &Path) -> Result<Staged, String> {
    let (root, temp) = if source.is_dir() {
        (source.to_path_buf(), None)
    } else if source.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
        let temp = std::env::temp_dir().join(format!("folio-install-{}", crate::safety::new_doc_id()));
        let unpacked = std::fs::File::open(source)
            .map_err(|e| e.to_string())
            .and_then(|f| zip::ZipArchive::new(f).map_err(|e| e.to_string()))
            // Keeps file permissions, like a program's executable bit.
            .and_then(|mut zip| zip.extract(&temp).map_err(|e| e.to_string()));
        if let Err(e) = unpacked {
            let _ = std::fs::remove_dir_all(&temp);
            return Err(format!("That zip couldn't be unpacked: {e}"));
        }
        (temp.clone(), Some(temp))
    } else {
        return Err("Choose an adapter's .zip file or its folder".into());
    };
    let mut staged = Staged {
        dir: root.clone(),
        manifest: Manifest::placeholder(),
        temp,
    };

    // The manifest is at the top, or one folder down (how zips usually come).
    let has_manifest = |d: &Path| d.join("adapter.toml").exists() || d.join("component.toml").exists();
    let dir = if has_manifest(&root) {
        root
    } else {
        let inner: Vec<PathBuf> = std::fs::read_dir(&root)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && has_manifest(p))
            .collect();
        match inner.as_slice() {
            [one] => one.clone(),
            [] => return Err("This isn't a Folio adapter: there's no adapter.toml in it".into()),
            _ => return Err("This holds more than one adapter; add them one at a time".into()),
        }
    };
    staged.manifest = load_manifest(&dir).map_err(|e| format!("Its adapter.toml has a problem: {e}"))?;
    staged.dir = dir;
    Ok(staged)
}

/// Installs a staged adapter into `root`, replacing any installed adapter
/// with the same name (the old copy is handed to `discard`). Returns where
/// it went.
pub fn install_into(staged: &Staged, root: &Path, discard: &dyn Fn(&Path) -> Result<(), String>) -> Result<PathBuf, String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    for existing in discover_in(&[root.to_path_buf()]) {
        if existing.manifest.name == staged.manifest.name {
            discard(&existing.dir)?;
        }
    }
    let base = staged
        .dir
        .file_name()
        .map_or("adapter".to_owned(), |n| n.to_string_lossy().into_owned());
    let mut dest = root.join(&base);
    let mut n = 2;
    while dest.exists() {
        dest = root.join(format!("{base}-{n}"));
        n += 1;
    }
    crate::platform::copy_dir(&staged.dir, &dest).map_err(|e| format!("Couldn't copy the adapter into place: {e}"))?;
    // Downloaded files are quarantined by macOS. The person chose to install
    // this adapter, and Folio still asks before it first runs.
    if cfg!(target_os = "macos") {
        let _ = Command::new("xattr")
            .args(["-dr", "com.apple.quarantine"])
            .arg(&dest)
            .status();
    }
    Ok(dest)
}

pub fn install(staged: &Staged) -> Result<PathBuf, String> {
    install_into(staged, &adapters_dir(), &crate::platform::move_to_trash)
}

/// Moves an installed adapter to the trash and forgets that it was allowed.
pub fn uninstall(adapter: &Adapter) -> Result<(), String> {
    crate::platform::move_to_trash(&adapter.dir)?;
    let mut approvals = load_approvals();
    if approvals.remove(&adapter.manifest.name).is_some()
        && let Some(path) = approvals_path()
    {
        let _ = std::fs::write(path, serde_json::to_string_pretty(&approvals).expect("serializes"));
    }
    Ok(())
}

impl Manifest {
    fn placeholder() -> Self {
        Self {
            name: String::new(),
            version: String::new(),
            author: String::new(),
            license: String::new(),
            description: String::new(),
            run: Vec::new(),
            permissions: Vec::new(),
            start: Start::Command,
            events: Vec::new(),
            commands: Vec::new(),
            panels: Vec::new(),
        }
    }
}

// ---- Approval ---------------------------------------------------------------
//
// The first time an adapter runs, Folio lists what it asks for and the user
// decides. The answer is remembered until the adapter asks for more.
//
// Permissions gate what an adapter can do *through Folio*. Adapters are
// ordinary programs, so `network` is a declaration, not a sandbox: only
// install adapters you trust, which is why they're open source.

pub fn describe(permission: &str) -> String {
    match permission {
        READ => "read the open document".into(),
        EDIT => "change the open document".into(),
        "network" => "connect to the internet or other computers".into(),
        other => other.to_owned(),
    }
}

fn approvals_path() -> Option<PathBuf> {
    Some(crate::settings::config_dir().join("approvals.json"))
}

fn load_approvals() -> BTreeMap<String, Vec<String>> {
    approvals_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Asks the user with a native dialog, remembering a yes.
pub fn approve_with_dialog(m: &Manifest) -> bool {
    let mut approvals = load_approvals();
    if let Some(granted) = approvals.get(&m.name)
        && m.permissions.iter().all(|p| granted.contains(p))
    {
        return true;
    }

    let by = if m.author.is_empty() { String::new() } else { format!(" by {}", m.author) };
    let mut text = format!("“{}” {}{by} wants to run.", m.name, m.version);
    if !m.license.is_empty() {
        text += &format!(" License: {}.", m.license);
    }
    if !m.description.is_empty() {
        text += &format!("\n\n{}", m.description);
    }
    if m.permissions.is_empty() {
        text += "\n\nIt doesn't ask for any permissions.";
    } else {
        text += "\n\nIt asks to:";
        for p in &m.permissions {
            text += &format!("\n•  {}", describe(p));
        }
    }
    text += "\n\nAdapters are separate programs. Only allow ones you trust.";

    let allowed = rfd::MessageDialog::new()
        .set_title("Allow adapter?")
        .set_description(text)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == rfd::MessageDialogResult::Yes;

    if allowed {
        approvals.insert(m.name.clone(), m.permissions.clone());
        if let Some(path) = approvals_path() {
            let _ = std::fs::create_dir_all(path.parent().expect("has parent"));
            let _ = std::fs::write(path, serde_json::to_string_pretty(&approvals).expect("serializes"));
        }
    }
    allowed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Writes an adapter folder with a Python program and manifest.
    fn write_adapter(dir: &Path, name: &str, script: &str, extra: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("main.py"), script).unwrap();
        std::fs::write(
            dir.join("adapter.toml"),
            format!(
                "name = \"{name}\"\nversion = \"1.0\"\nrun = [\"python3\", \"main.py\"]\n\
                 permissions = [\"document.read\", \"document.edit\"]\n{extra}"
            ),
        )
        .unwrap();
    }

    /// Zips a folder as one top-level folder, the way folio-adapters does.
    /// The script is marked executable, as a compiled adapter would be.
    fn zip_folder(dir: &Path, zip: &Path) {
        use zip::write::SimpleFileOptions;
        let mut out = zip::ZipWriter::new(std::fs::File::create(zip).unwrap());
        let top = dir.file_name().unwrap().to_string_lossy().into_owned();
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let name = format!("{top}/{}", entry.file_name().to_string_lossy());
            let mode = if name.ends_with(".py") { 0o755 } else { 0o644 };
            out.start_file(name, SimpleFileOptions::default().unix_permissions(mode)).unwrap();
            out.write_all(&std::fs::read(entry.path()).unwrap()).unwrap();
        }
        out.finish().unwrap();
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("folio-{name}-{}", crate::safety::new_doc_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const ROUND_TRIP: &str = r#"
import json, sys
def send(m): print(json.dumps(m), flush=True)
for line in sys.stdin:
    m = json.loads(line)
    if m.get("method") == "command" and m["params"]["command"] == "date":
        send({"id": 1, "method": "doc.insert", "params": {"text": "October 24, 2026"}})
    elif m.get("method") == "command" and m["params"]["command"] == "goal":
        send({"id": 2, "method": "doc.text", "params": {}})
    elif m.get("id") == 2:
        words = len(m["result"]["text"].split())
        send({"method": "status.set", "params": {"text": f"{words} / 250 words"}})
        send({"method": "panel.set", "params": {"id": "goal", "lines": [
            f"{words} of 250 words", {"text": "Next goal", "command": "goal"}]}})
"#;

    /// A full round trip with a real adapter process: command in, request
    /// out, reply in, status and panel out.
    #[test]
    fn adapter_round_trip() {
        let root = temp("round-trip");
        write_adapter(&root.join("tools"), "Tools", ROUND_TRIP, "");
        let mut host = Host::new(discover_in(std::slice::from_ref(&root)), Arc::new(|| {}), Box::new(|_| true));

        host.run_command("Tools", "date").unwrap();
        let call = wait_for(&mut host, |c| c.method == "doc.insert");
        assert_eq!(call.params["text"], "October 24, 2026");
        host.reply(0, call.id.unwrap(), Ok(Value::Null));

        host.run_command("Tools", "goal").unwrap();
        let call = wait_for(&mut host, |c| c.method == "doc.text");
        host.reply(0, call.id.unwrap(), Ok(json!({ "text": "one two three" })));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !host.all[0].panels.contains_key("goal") && Instant::now() < deadline {
            host.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(host.all[0].status.as_deref(), Some("3 / 250 words"));
        let panel = &host.all[0].panels["goal"];
        assert_eq!(panel[0].text, "3 of 250 words");
        assert_eq!(panel[1].command.as_deref(), Some("goal"), "clickable lines run commands");
        drop(host);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn installs_from_zips_and_folders() {
        let downloads = temp("downloads");
        let (root, trash) = (temp("installed"), temp("trash"));

        let discard = |d: &Path| crate::platform::move_into(d, &trash);

        // A zip holding one folder, the way folio-adapters ships them.
        write_adapter(&downloads.join("tools"), "Tools", ROUND_TRIP, "");
        let zip = downloads.join("tools-1.0.zip");
        zip_folder(&downloads.join("tools"), &zip);
        let staged = stage(&zip).unwrap();
        assert_eq!(staged.manifest.name, "Tools");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(staged.dir.join("main.py")).unwrap().permissions().mode();
            assert!(mode & 0o100 != 0, "the executable bit survives the zip");
        }
        let dest = install_into(&staged, &root, &discard).unwrap();
        assert_eq!(dest, root.join("tools"));
        drop(staged);
        assert_eq!(discover_in(std::slice::from_ref(&root)).len(), 1);

        // Installing a newer copy from a folder replaces it; the old one goes to the trash.
        let newer = downloads.join("newer/tools");
        write_adapter(&newer, "Tools", ROUND_TRIP, "description = \"v2\"");
        let staged = stage(&downloads.join("newer")).unwrap();
        install_into(&staged, &root, &discard).unwrap();
        let found = discover_in(std::slice::from_ref(&root));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].manifest.description, "v2");
        assert!(trash.join("tools").exists(), "the old copy can be recovered");

        // Not an adapter, or a broken one, is refused with a reason.
        let empty = downloads.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(stage(&empty).err().unwrap().contains("no adapter.toml"));
        let broken = downloads.join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("adapter.toml"), "name = ").unwrap();
        assert!(stage(&broken).err().unwrap().contains("problem"));
        assert!(stage(&downloads.join("tools-1.0.txt")).is_err());

        for d in [downloads, root, trash] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn sync_keeps_running_adapters_and_stops_removed_ones() {
        let root = temp("sync");
        write_adapter(&root.join("a"), "A", ROUND_TRIP, "");
        write_adapter(&root.join("b"), "B", ROUND_TRIP, "");
        let mut host = Host::new(discover_in(std::slice::from_ref(&root)), Arc::new(|| {}), Box::new(|_| true));
        host.run_command("A", "goal").unwrap();
        assert!(host.all[0].is_running());

        // B is uninstalled and C installed while Folio is open.
        crate::platform::move_into(&root.join("b"), &root.join(".trash")).unwrap();
        write_adapter(&root.join("c"), "C", ROUND_TRIP, "");
        host.sync(discover_in(std::slice::from_ref(&root)));

        let names: Vec<&str> = host.installed().map(|(_, a)| a.manifest.name.as_str()).collect();
        assert_eq!(names, ["A", "C"]);
        assert!(host.all[0].is_running(), "A kept running");
        assert!(host.all[1].removed && !host.all[1].is_running());
        assert_eq!(host.index_of("C"), Some(2), "indexes never shift");
        assert_eq!(host.index_of("B"), None);
        drop(host);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A scripted adapter that starts a session, shows a peer, and reports
    /// back when Folio routes undo to it.
    #[test]
    fn sessions_presence_and_undo_routing() {
        let dir = std::env::temp_dir().join(format!("folio-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("fake.py"),
            r##"
import json, sys
def send(m): print(json.dumps(m), flush=True)
for line in sys.stdin:
    m = json.loads(line)
    if m.get("method") == "command":
        send({"method": "session.set", "params": {"active": True, "handles_undo": True}})
        send({"method": "presence.set", "params": {"peers": [
            {"name": "Ana", "color": "#ff0000", "cursor": {"para": 0, "ch": 3}}]}})
    elif m.get("method") == "undo":
        send({"method": "status.set", "params": {"text": "undid"}})
    elif m.get("method") == "event" and m["params"]["kind"] == "snapshot":
        send({"method": "status.set", "params": {"text": "rev " + str(m["params"]["revision"])}})
"##,
        )
        .unwrap();
        let manifest = Manifest {
            name: "Fake".into(),
            version: "1".into(),
            author: String::new(),
            license: String::new(),
            description: String::new(),
            run: vec!["python3".into(), "fake.py".into()],
            permissions: vec![READ.into()],
            start: Start::Command,
            events: vec!["snapshot".into()],
            commands: Vec::new(),
            panels: Vec::new(),
        };
        let adapter = Adapter {
            manifest,
            dir: dir.clone(),
            running: None,
            generation: 0,
            status: None,
            panels: BTreeMap::new(),
            blocked: false,
            removed: false,
        };
        let mut host = Host::new(vec![adapter], Arc::new(|| {}), Box::new(|_| true));
        host.run_command("Fake", "share").unwrap();

        let until = |host: &mut Host, done: &dyn Fn(&Host) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !done(host) && Instant::now() < deadline {
                host.poll();
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        until(&mut host, &|h| !h.peers.is_empty());
        assert_eq!(host.session, Some((0, true)));
        assert_eq!(host.peers[0].name, "Ana");
        assert_eq!(host.peers[0].anchor, Pos::new(0, 3), "anchor defaults to the cursor");

        host.notify(0, "undo");
        until(&mut host, &|h| h.all[0].status.as_deref() == Some("undid"));
        assert_eq!(host.all[0].status.as_deref(), Some("undid"));

        assert!(host.wants("snapshot"));
        host.emit("snapshot", json!({ "revision": 7 }));
        until(&mut host, &|h| h.all[0].status.as_deref() == Some("rev 7"));
        assert_eq!(host.all[0].status.as_deref(), Some("rev 7"));
        drop(host);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Installs the real zips built by the folio-adapters repo and exports a
    /// PDF through the PDF Export adapter, the way the app does:
    /// `FOLIO_ADAPTERS_DIST=../folio-adapters/dist FOLIO_MODEL=model.json
    ///  FOLIO_PDF_OUT=out.pdf cargo test real_adapter_zips -- --ignored`
    #[test]
    #[ignore]
    fn real_adapter_zips() {
        let dist = PathBuf::from(std::env::var("FOLIO_ADAPTERS_DIST").unwrap());
        let (root, trash) = (temp("real-installed"), temp("real-trash"));
        let mut zips: Vec<PathBuf> = std::fs::read_dir(&dist)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "zip"))
            .collect();
        zips.sort();
        for zip in &zips {
            install_into(&stage(zip).unwrap(), &root, &|d: &Path| crate::platform::move_into(d, &trash)).unwrap();
        }
        let found = discover_in(std::slice::from_ref(&root));
        let mut names: Vec<&str> = found.iter().map(|a| a.manifest.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["Folio Link", "PDF Export", "Writing Tools"]);

        let mut host = Host::new(found, Arc::new(|| {}), Box::new(|_| true));
        let pdf = host.index_of("PDF Export").unwrap();
        host.run_command("PDF Export", "export").unwrap();
        let call = wait_for(&mut host, |c| c.method == "doc.print_model");
        let model: Value = serde_json::from_str(&std::fs::read_to_string(std::env::var("FOLIO_MODEL").unwrap()).unwrap()).unwrap();
        host.reply(pdf, call.id.unwrap(), Ok(model));
        let call = wait_for(&mut host, |c| c.method == "ui.save_dialog");
        let out = std::env::var("FOLIO_PDF_OUT").unwrap();
        host.reply(pdf, call.id.unwrap(), Ok(json!(out)));
        let call = wait_for(&mut host, |c| c.method == "notify");
        assert!(call.params["text"].as_str().unwrap().starts_with("Exported"), "{:?}", call.params);
        assert!(std::fs::metadata(&out).unwrap().len() > 1000);
        drop(host);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(trash);
    }

    #[test]
    fn old_components_folders_still_load() {
        let root = std::env::temp_dir().join(format!("folio-adapters-{}", std::process::id()));
        let old = root.join("components/legacy");
        let new = root.join("adapters/modern");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("component.toml"), "name = \"Legacy\"\nversion = \"1\"\nrun = [\"x\"]").unwrap();
        std::fs::write(new.join("adapter.toml"), "name = \"Modern\"\nversion = \"1\"\nrun = [\"x\"]").unwrap();

        let found = discover_in(&[root.join("components"), root.join("adapters")]);
        let names: Vec<&str> = found.iter().map(|a| a.manifest.name.as_str()).collect();
        assert_eq!(names, ["Legacy", "Modern"]);
        let _ = std::fs::remove_dir_all(root);
    }

    fn wait_for(host: &mut Host, pred: impl Fn(&Call) -> bool) -> Call {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let (calls, errors) = host.poll();
            assert!(errors.is_empty(), "{errors:?}");
            if let Some(call) = calls.into_iter().find(|c| pred(c)) {
                return call;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("adapter didn't send the expected message");
    }
}
