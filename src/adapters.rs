//! Adapters: optional add-ons that run as separate programs, in the spirit
//! of foobar2000's components. Folio stays bare; everything extra is an
//! adapter someone installs on purpose.
//!
//! An adapter is a folder holding `adapter.toml` and a program. Folio
//! starts the program only when it's needed and talks to it with one JSON
//! message per line over stdin/stdout. See `docs/adapters.md` for the
//! protocol.
//!
//! Folio looks for adapters in `./adapters` and in
//! `~/Library/Application Support/Folio/adapters`. Adapters used to be called
//! components; `components/` folders and `component.toml` still work.

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
    /// Put the command in Folio's "file", "edit", "format" or "view" menu
    /// instead of the adapter's own submenu under Adapters.
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
        self.all.iter().position(|c| c.manifest.name == name)
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
        let (program, args) = c.manifest.run.split_first().ok_or("adapter.toml has an empty `run`")?;
        let mut child = Command::new(program)
            .args(args)
            .current_dir(&c.dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("Couldn't start {}: {e}", c.manifest.name))?;

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
        self.all.iter().filter_map(|c| c.status.as_deref())
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

// ---- Discovery --------------------------------------------------------------

pub fn discover() -> Vec<Adapter> {
    let config = crate::settings::config_dir();
    discover_in(&[
        PathBuf::from("components"),
        PathBuf::from("adapters"),
        config.join("components"),
        config.join("adapters"),
    ])
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

    /// Starts the bundled sample adapter and checks a full round trip:
    /// command in, request out, reply in, follow-up message out.
    #[test]
    fn sample_adapter_round_trip() {
        let adapters: Vec<Adapter> = discover()
            .into_iter()
            .filter(|c| c.manifest.name == "Writing Tools")
            .collect();
        assert_eq!(adapters.len(), 1, "sample adapter is in ./adapters");

        let mut host = Host::new(adapters, Arc::new(|| {}), Box::new(|_| true));
        host.run_command("Writing Tools", "date").unwrap();

        let call = wait_for(&mut host, |c| c.method == "doc.insert");
        assert!(call.params["text"].as_str().unwrap().len() >= 8);
        host.reply(0, call.id.unwrap(), Ok(Value::Null));

        // The goal command answers through the status line, which the host handles itself.
        host.run_command("Writing Tools", "goal").unwrap();
        let call = wait_for(&mut host, |c| c.method == "doc.text");
        host.reply(0, call.id.unwrap(), Ok(json!({ "text": "one two three" })));
        let deadline = Instant::now() + Duration::from_secs(5);
        let has_progress = |h: &Host| {
            h.all[0].panels.get("goal").is_some_and(|p| p.iter().any(|l| l.text.contains("3 of")))
        };
        while !(host.all[0].status.is_some() && has_progress(&host)) && Instant::now() < deadline {
            host.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(host.all[0].status.as_deref(), Some("3 / 250 words"));

        // Its panel shows progress, with a clickable line that runs a command.
        let panel = &host.all[0].panels["goal"];
        assert!(panel.iter().any(|l| l.text.contains("3 of 250")), "{panel:?}");
        assert!(panel.iter().any(|l| l.command.as_deref() == Some("goal")));
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

    #[test]
    fn shipped_adapters_load() {
        let found = discover_in(&[PathBuf::from("adapters")]);
        let mut names: Vec<&str> = found.iter().map(|a| a.manifest.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["Folio Link", "PDF Export", "Writing Tools"]);
        let pdf = found.iter().find(|a| a.manifest.name == "PDF Export").unwrap();
        assert_eq!(pdf.manifest.commands[0].menu.as_deref(), Some("file"));
        let link = found.iter().find(|a| a.manifest.name == "Folio Link").unwrap();
        assert!(link.manifest.events.iter().any(|e| e == "snapshot"));
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
