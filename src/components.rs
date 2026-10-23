//! Components: optional add-ons that run as separate programs, in the spirit
//! of foobar2000's components. Folio stays bare; everything extra is a
//! component someone installs on purpose.
//!
//! A component is a folder holding `component.toml` and a program. Folio
//! starts the program only when it's needed and talks to it with one JSON
//! message per line over stdin/stdout. See `docs/components.md` for the
//! protocol.
//!
//! Folio looks for components in `./components` and in
//! `~/Library/Application Support/Folio/components`.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use serde::Deserialize;
use serde_json::{Value, json};

pub const PROTOCOL_VERSION: u32 = 1;

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
    /// Program and arguments, run from the component's folder.
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
    /// Menu shortcut, e.g. "Cmd+Shift+D".
    pub keys: Option<String>,
}

/// A message from a component that the app has to act on.
#[derive(Debug)]
pub struct Call {
    pub component: usize,
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

pub struct Component {
    pub manifest: Manifest,
    dir: PathBuf,
    running: Option<Running>,
    generation: u64,
    /// Text the component shows in the status line.
    pub status: Option<String>,
}

pub type Waker = Arc<dyn Fn() + Send + Sync>;
pub type Approver = Box<dyn Fn(&Manifest) -> bool>;

pub struct Host {
    pub components: Vec<Component>,
    tx: Sender<(usize, u64, Incoming)>,
    rx: Receiver<(usize, u64, Incoming)>,
    waker: Waker,
    approver: Approver,
    launched: bool,
}

impl Host {
    /// `waker` is called from background threads when a component sends
    /// something, so an idle app can wake up and handle it.
    pub fn new(components: Vec<Component>, waker: Waker, approver: Approver) -> Self {
        let (tx, rx) = channel();
        Self {
            components,
            tx,
            rx,
            waker,
            approver,
            launched: false,
        }
    }

    pub fn has_permission(&self, i: usize, permission: &str) -> bool {
        self.components[i].manifest.permissions.iter().any(|p| p == permission)
    }

    fn start(&mut self, i: usize) -> Result<(), String> {
        if self.components[i].running.is_some() {
            return Ok(());
        }
        if !(self.approver)(&self.components[i].manifest) {
            return Err(format!("{} wasn't allowed to run", self.components[i].manifest.name));
        }

        let c = &mut self.components[i];
        let (program, args) = c.manifest.run.split_first().ok_or("component.toml has an empty `run`")?;
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
        if let Some(r) = &mut self.components[i].running {
            let line = format!("{message}\n");
            // A component that has quit just misses the message.
            let _ = r.stdin.write_all(line.as_bytes()).and_then(|()| r.stdin.flush());
        }
    }

    /// Runs one of a component's menu commands, starting it if needed.
    pub fn run_command(&mut self, i: usize, command: usize) -> Result<(), String> {
        self.start(i)?;
        let id = self.components[i].manifest.commands[command].id.clone();
        self.send(i, json!({ "method": "command", "params": { "command": id } }));
        Ok(())
    }

    /// Sends an event to every running component that asked for it.
    pub fn emit(&mut self, event: &str, params: Value) {
        for i in 0..self.components.len() {
            let c = &self.components[i];
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

    /// Starts `start = "launch"` components (once), handles messages Folio can
    /// answer on its own, and returns the rest for the app.
    pub fn poll(&mut self) -> (Vec<Call>, Vec<String>) {
        let mut errors = Vec::new();
        if !self.launched {
            self.launched = true;
            for i in 0..self.components.len() {
                if self.components[i].manifest.start == Start::Launch
                    && let Err(e) = self.start(i)
                {
                    errors.push(e);
                }
            }
        }

        let mut calls = Vec::new();
        while let Ok((i, generation, incoming)) = self.rx.try_recv() {
            // Ignore leftovers from an earlier run of the same component.
            if self.components[i].running.as_ref().map(|r| r.generation) != Some(generation) {
                continue;
            }
            match incoming {
                Incoming::Exited => {
                    let c = &mut self.components[i];
                    if let Some(mut r) = c.running.take() {
                        let _ = r.child.wait();
                    }
                    c.status = None;
                }
                Incoming::Line(line) => {
                    let message: Value = match serde_json::from_str(&line) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("folio: {} sent invalid JSON: {e}", self.components[i].manifest.name);
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
                            let c = &mut self.components[i];
                            c.status = params.get("text").and_then(Value::as_str).map(str::to_owned);
                            if let Some(id) = id {
                                self.reply(i, id, Ok(Value::Null));
                            }
                        }
                        "log" => eprintln!(
                            "[{}] {}",
                            self.components[i].manifest.name,
                            params.get("message").and_then(Value::as_str).unwrap_or_default()
                        ),
                        _ => calls.push(Call {
                            component: i,
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
        self.components.iter().filter_map(|c| c.status.as_deref())
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        for i in 0..self.components.len() {
            self.send(i, json!({ "method": "shutdown" }));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        for c in &mut self.components {
            if let Some(mut r) = c.running.take() {
                let _ = r.child.kill();
                let _ = r.child.wait();
            }
        }
    }
}

// ---- Discovery --------------------------------------------------------------

fn support_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/Folio"))
}

pub fn discover() -> Vec<Component> {
    let mut dirs = vec![PathBuf::from("components")];
    if let Some(support) = support_dir() {
        dirs.push(support.join("components"));
    }

    let mut found: Vec<Component> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut folders: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        folders.sort();
        for folder in folders {
            match load_manifest(&folder) {
                // An installed component replaces a bundled one with the same name.
                Ok(manifest) => {
                    found.retain(|c| c.manifest.name != manifest.name);
                    found.push(Component {
                        manifest,
                        dir: folder,
                        running: None,
                        generation: 0,
                        status: None,
                    });
                }
                Err(e) => eprintln!("folio: skipping component {}: {e}", folder.display()),
            }
        }
    }
    found
}

fn load_manifest(folder: &Path) -> Result<Manifest, String> {
    let src = std::fs::read_to_string(folder.join("component.toml")).map_err(|e| e.to_string())?;
    toml::from_str(&src).map_err(|e| e.to_string())
}

// ---- Approval ---------------------------------------------------------------
//
// The first time a component runs, Folio lists what it asks for and the user
// decides. The answer is remembered until the component asks for more.
//
// Permissions gate what a component can do *through Folio*. Components are
// ordinary programs, so `network` is a declaration, not a sandbox: only
// install components you trust, which is why they're open source.

pub fn describe(permission: &str) -> String {
    match permission {
        READ => "read the open document".into(),
        EDIT => "change the open document".into(),
        "network" => "connect to the internet or other computers".into(),
        other => other.to_owned(),
    }
}

fn approvals_path() -> Option<PathBuf> {
    support_dir().map(|d| d.join("approvals.json"))
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
    text += "\n\nComponents are separate programs. Only allow ones you trust.";

    let allowed = rfd::MessageDialog::new()
        .set_title("Allow component?")
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

    /// Starts the bundled sample component and checks a full round trip:
    /// command in, request out, reply in, follow-up message out.
    #[test]
    fn sample_component_round_trip() {
        let components: Vec<Component> = discover()
            .into_iter()
            .filter(|c| c.manifest.name == "Writing Tools")
            .collect();
        assert_eq!(components.len(), 1, "sample component is in ./components");

        let mut host = Host::new(components, Arc::new(|| {}), Box::new(|_| true));
        let date = host.components[0].manifest.commands.iter().position(|c| c.id == "date").unwrap();
        host.run_command(0, date).unwrap();

        let call = wait_for(&mut host, |c| c.method == "doc.insert");
        assert!(call.params["text"].as_str().unwrap().len() >= 8);
        host.reply(0, call.id.unwrap(), Ok(Value::Null));

        // The goal command answers through the status line, which the host handles itself.
        let goal = host.components[0].manifest.commands.iter().position(|c| c.id == "goal").unwrap();
        host.run_command(0, goal).unwrap();
        let call = wait_for(&mut host, |c| c.method == "doc.text");
        host.reply(0, call.id.unwrap(), Ok(json!({ "text": "one two three" })));
        let deadline = Instant::now() + Duration::from_secs(5);
        while host.components[0].status.is_none() && Instant::now() < deadline {
            host.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(host.components[0].status.as_deref(), Some("3 / 250 words"));
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
        panic!("component didn't send the expected message");
    }
}
