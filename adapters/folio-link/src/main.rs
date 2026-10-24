//! Folio Link: live co-editing of a Folio document over encrypted
//! peer-to-peer links. Speaks Folio's adapter protocol (v3) on stdin/stdout;
//! see docs/adapters.md and docs/folio-link.md.

mod crdt;
mod net;

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, Write};

use iroh::Endpoint;
use loro::cursor::Cursor;
use serde_json::{json, Value};
use tokio::sync::mpsc::{self, UnboundedSender};

use crdt::{Crdt, Paragraph, Pos};
use net::{Frame, Invite, NetEvent, Presence, Settings};

const COLORS: [&str; 8] = [
    "#e5484d", "#3e63dd", "#30a46c", "#f76b15", "#8e4ec6", "#12a594", "#d6409f", "#c28a00",
];

fn color_for(who: &str) -> String {
    let hash = who
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    COLORS[hash as usize % COLORS.len()].into()
}

enum Input {
    Folio(Value),
    Net(NetEvent),
    Hosted(anyhow::Result<(Endpoint, Invite)>),
    Joined(anyhow::Result<(Endpoint, String, UnboundedSender<Frame>)>),
    Eof,
}

/// What a request to Folio was for, so its reply can be acted on.
enum Pending {
    ShareDoc,
    ConfirmJoin,
    Admit(u64),
    Merge {
        frontier: loro::Frontiers,
        base: u64,
    },
    Ignore,
}

#[derive(Clone)]
struct Snapshot {
    revision: u64,
    paragraphs: Vec<Paragraph>,
    cursor: Pos,
    anchor: Pos,
}

struct Guest {
    tx: UnboundedSender<Frame>,
    who: String,
    name: String,
    admitted: bool,
}

enum Role {
    Host {
        invite: String,
        doc_id: String,
        secret: String,
        guests: HashMap<u64, Guest>,
    },
    Guest {
        host: UnboundedSender<Frame>,
        host_who: String,
        host_name: Option<String>,
    },
}

struct Session {
    endpoint: Endpoint,
    role: Role,
    /// None until a guest has the host's document.
    crdt: Option<Crdt>,
    /// Folio's revision that matches the CRDT's shadow version.
    revision: Option<u64>,
    /// Folio shows the shared document. A guest's window doesn't until its
    /// first merge lands.
    on_screen: bool,
    merging: bool,
    caret: Option<(Cursor, Cursor)>,
    peers: BTreeMap<String, Presence>,
    sent_presence: Value,
}

struct App {
    next_id: u64,
    pending: HashMap<u64, Pending>,
    settings: Settings,
    name: String,
    snapshot: Option<Snapshot>,
    session: Option<Session>,
    /// Set while Share or Join is starting up.
    starting: bool,
    /// The document to host once the endpoint is up.
    session_seed: Option<Crdt>,
    /// The invite to use once joining is confirmed.
    join_invite: Option<Invite>,
    inputs: UnboundedSender<Input>,
}

fn send(message: Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{message}");
    let _ = out.flush();
}

fn notify_folio(method: &str, params: Value) {
    send(json!({"method": method, "params": params}));
}

fn pos(v: &Value) -> Pos {
    serde_json::from_value(v.clone()).unwrap_or_default()
}

fn pos_json(p: Pos) -> Value {
    json!({"para": p.para, "ch": p.ch})
}

/// The invite lives on the clipboard. `FOLIO_LINK_CLIPBOARD` names a file to
/// use instead, for tests.
fn copy(text: &str) -> bool {
    if let Some(path) = std::env::var_os("FOLIO_LINK_CLIPBOARD") {
        return std::fs::write(path, text).is_ok();
    }
    let Ok(mut child) = std::process::Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
    else {
        return false;
    };
    let wrote = child
        .stdin
        .take()
        .map(|mut s| s.write_all(text.as_bytes()).is_ok())
        .unwrap_or(false);
    child.wait().map(|s| s.success()).unwrap_or(false) && wrote
}

fn paste() -> String {
    if let Some(path) = std::env::var_os("FOLIO_LINK_CLIPBOARD") {
        return std::fs::read_to_string(path).unwrap_or_default();
    }
    std::process::Command::new("pbpaste")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default()
}

fn people(n: usize) -> String {
    if n == 1 {
        "1 person".into()
    } else {
        format!("{n} people")
    }
}

impl App {
    fn request(&mut self, method: &str, params: Value, pending: Pending) {
        self.next_id += 1;
        self.pending.insert(self.next_id, pending);
        send(json!({"id": self.next_id, "method": method, "params": params}));
    }

    fn notify(&self, text: &str) {
        notify_folio("notify", json!({"text": text}));
    }

    fn handle_folio(&mut self, msg: Value) {
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match msg.get("method").and_then(Value::as_str) {
            Some("initialize") => {}
            Some("command") => match params["command"].as_str().unwrap_or("") {
                "share" => self.share(),
                "join" => self.join(),
                "stop" => {
                    self.stop(true);
                }
                "copy-invite" => self.copy_invite(),
                other => eprintln!("folio-link: unknown command {other:?}"),
            },
            Some("event") if params["kind"] == "snapshot" => self.on_snapshot(&params),
            Some("undo") => self.undo_redo(true),
            Some("redo") => self.undo_redo(false),
            Some("shutdown") => self.quit(),
            Some(_) => {}
            None => {
                let Some(id) = msg.get("id").and_then(Value::as_u64) else {
                    return;
                };
                if let Some(pending) = self.pending.remove(&id) {
                    match msg.get("error") {
                        Some(e) if !e.is_null() => self.on_error(pending, e),
                        _ => self.on_reply(pending, msg.get("result").cloned().unwrap_or_default()),
                    }
                }
            }
        }
    }

    fn on_error(&mut self, pending: Pending, error: &Value) {
        eprintln!("folio-link: request failed: {error}");
        match pending {
            Pending::ShareDoc => {
                self.starting = false;
                self.notify("Couldn't read the document to share it.");
            }
            Pending::Admit(peer) => self.admit(peer, false),
            Pending::Merge { .. } => {
                if let Some(s) = &mut self.session {
                    s.merging = false;
                }
            }
            Pending::ConfirmJoin => self.starting = false,
            Pending::Ignore => {}
        }
    }

    fn on_reply(&mut self, pending: Pending, result: Value) {
        match pending {
            Pending::ShareDoc => {
                let paragraphs: Vec<Paragraph> =
                    serde_json::from_value(result["paragraphs"].clone()).unwrap_or_default();
                self.start_hosting(paragraphs);
            }
            Pending::ConfirmJoin => {
                if result.as_bool() == Some(true) {
                    self.connect();
                } else {
                    self.starting = false;
                }
            }
            Pending::Admit(peer) => self.admit(peer, result.as_bool() == Some(true)),
            Pending::Merge { frontier, base } => {
                let applied = result["applied"].as_bool() == Some(true);
                let Some(s) = &mut self.session else { return };
                s.merging = false;
                let Some(crdt) = &mut s.crdt else { return };
                if applied {
                    crdt.set_shadow(frontier);
                    if !s.on_screen {
                        // A guest's first merge: the shared document is now on screen.
                        s.on_screen = true;
                        s.revision = Some(base);
                        self.status();
                    }
                    self.push_presence();
                } else if self.snapshot.as_ref().map(|x| x.revision) != Some(base) {
                    // Folio moved on before our merge got there; try again.
                    self.merge_if_behind();
                }
            }
            Pending::Ignore => {}
        }
    }

    // ---- commands ----

    fn share(&mut self) {
        if self.session.is_some() || self.starting {
            self.notify("Folio Link is already on. Choose Stop Sharing first.");
            return;
        }
        self.starting = true;
        self.request("doc.get", json!({}), Pending::ShareDoc);
    }

    fn start_hosting(&mut self, paragraphs: Vec<Paragraph>) {
        let crdt = Crdt::from_paragraphs(&paragraphs);
        let settings = self.settings.clone();
        let inputs = self.inputs.clone();
        // Keep the document until the endpoint is up.
        self.session_seed = Some(crdt);
        tokio::spawn(async move {
            let result = async {
                let key = net::load_key()?;
                let endpoint = net::bind(&settings, key, true).await?;
                let addr = net::ready_addr(&endpoint, &settings.relay).await;
                let invite = Invite {
                    addr,
                    doc: net::random_hex(8),
                    secret: net::random_hex(16),
                };
                Ok((endpoint, invite))
            }
            .await;
            let _ = inputs.send(Input::Hosted(result));
        });
    }

    fn on_hosted(&mut self, result: anyhow::Result<(Endpoint, Invite)>) {
        self.starting = false;
        let crdt = self.session_seed.take();
        let (endpoint, invite) = match result {
            Ok(r) => r,
            Err(e) => {
                self.notify(&format!("Couldn't start sharing: {e}"));
                return;
            }
        };
        let Some(crdt) = crdt else { return };
        let text = invite.encode();
        tokio::spawn(net::accept_loop(endpoint.clone(), self.net_sender()));
        self.session = Some(Session {
            endpoint,
            role: Role::Host {
                invite: text.clone(),
                doc_id: invite.doc,
                secret: invite.secret,
                guests: HashMap::new(),
            },
            crdt: Some(crdt),
            revision: None,
            on_screen: true,
            merging: false,
            caret: None,
            peers: BTreeMap::new(),
            sent_presence: Value::Null,
        });
        self.set_session(true);
        self.status();
        if copy(&text) {
            self.notify("Invite copied. Send it to the people you want to edit with.");
        } else {
            self.notify("Sharing, but the invite couldn't be copied.");
        }
    }

    fn join(&mut self) {
        if self.session.is_some() || self.starting {
            self.notify("Folio Link is already on. Choose Stop Sharing first.");
            return;
        }
        let Ok(invite) = Invite::decode(&paste()) else {
            self.notify("The clipboard doesn't hold a Folio Link invite.");
            return;
        };
        self.join_invite = Some(invite);
        self.starting = true;
        let has_text = self.snapshot.as_ref().is_some_and(|s| {
            s.paragraphs.len() > 1
                || s.paragraphs
                    .iter()
                    .any(|p| p.runs.iter().any(|r| !r.text.is_empty()))
        });
        if has_text {
            self.request(
                "ui.confirm",
                json!({
                    "title": "Join a shared document?",
                    "message": "Joining replaces what's in this window with the shared document.",
                }),
                Pending::ConfirmJoin,
            );
        } else {
            self.connect();
        }
    }

    fn connect(&mut self) {
        let Some(invite) = self.join_invite.take() else {
            self.starting = false;
            return;
        };
        let settings = self.settings.clone();
        let inputs = self.inputs.clone();
        let events = self.net_sender();
        let hello = Frame::Hello {
            doc: invite.doc.clone(),
            secret: invite.secret.clone(),
            name: self.name.clone(),
        };
        notify_folio("status.set", json!({"text": "● Connecting…"}));
        tokio::spawn(async move {
            let result = async {
                let key = net::load_key()?;
                let endpoint = net::bind(&settings, key, false).await?;
                let (who, tx) = net::connect(&endpoint, &invite, hello, events).await?;
                Ok((endpoint, who, tx))
            }
            .await;
            let _ = inputs.send(Input::Joined(result));
        });
    }

    fn on_joined(&mut self, result: anyhow::Result<(Endpoint, String, UnboundedSender<Frame>)>) {
        self.starting = false;
        let (endpoint, host_who, host) = match result {
            Ok(r) => r,
            Err(e) => {
                notify_folio("status.set", json!({"text": null}));
                self.notify(&format!("Couldn't join: {e}"));
                return;
            }
        };
        notify_folio("status.set", json!({"text": "● Waiting to be let in…"}));
        self.session = Some(Session {
            endpoint,
            role: Role::Guest {
                host,
                host_who,
                host_name: None,
            },
            crdt: None,
            revision: None,
            on_screen: false,
            merging: false,
            caret: None,
            peers: BTreeMap::new(),
            sent_presence: Value::Null,
        });
    }

    fn copy_invite(&mut self) {
        match &self.session {
            Some(Session {
                role: Role::Host { invite, .. },
                ..
            }) => {
                if copy(invite) {
                    self.notify("Invite copied.");
                }
            }
            _ => self.notify("You're not sharing a document right now."),
        }
    }

    /// Leaves or stops hosting and clears everything Folio shows for it.
    /// Returns the task closing the endpoint.
    fn stop(&mut self, tell: bool) -> Option<tokio::task::JoinHandle<()>> {
        let Some(session) = self.session.take() else {
            if tell {
                self.notify("Folio Link isn't on.");
            }
            return None;
        };
        let hosting = matches!(session.role, Role::Host { .. });
        // Replies to this session's requests mean nothing to the next one.
        self.pending
            .retain(|_, p| !matches!(p, Pending::Merge { .. } | Pending::Admit(_)));
        let endpoint = session.endpoint.clone();
        drop(session);
        // Give dropped connections a moment to say goodbye.
        let close = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            endpoint.close().await;
        });
        self.clear_ui();
        if tell {
            self.notify(if hosting {
                "Stopped sharing."
            } else {
                "Left the shared document."
            });
        }
        Some(close)
    }

    /// Stops, waiting briefly so peers hear we left, then exits.
    fn quit(&mut self) -> ! {
        if let Some(close) = self.stop(false) {
            futures_block(close);
        }
        std::process::exit(0);
    }

    fn clear_ui(&mut self) {
        self.set_session(false);
        notify_folio("presence.set", json!({"peers": []}));
        notify_folio("status.set", json!({"text": null}));
        notify_folio(
            "panel.set",
            json!({"id": "people", "lines": ["Not sharing.",
            {"text": "Share this document →", "command": "share"},
            {"text": "Join from clipboard →", "command": "join"}]}),
        );
    }

    fn set_session(&mut self, active: bool) {
        self.request(
            "session.set",
            json!({"active": active, "handles_undo": active}),
            Pending::Ignore,
        );
    }

    fn status(&mut self) {
        let Some(s) = &self.session else { return };
        let me = format!("● You ({})", self.name);
        let (text, mut lines) = match &s.role {
            Role::Host { guests, .. } => {
                let mut lines = vec![json!(me)];
                for g in guests.values().filter(|g| g.admitted) {
                    lines.push(json!(format!("● {}", g.name)));
                }
                let n = lines.len();
                (format!("● Sharing · {}", people(n)), lines)
            }
            Role::Guest {
                host_name,
                host_who,
                ..
            } => {
                let host = host_name.clone().unwrap_or_else(|| "the host".into());
                let mut lines = vec![json!(format!("● {host} (host)")), json!(me)];
                for p in s.peers.values().filter(|p| &p.who != host_who) {
                    lines.push(json!(format!("● {}", p.name)));
                }
                (format!("● Joined {host}'s document"), lines)
            }
        };
        if matches!(s.role, Role::Host { .. }) {
            lines.push(json!({"text": "Copy invite again →", "command": "copy-invite"}));
            lines.push(json!({"text": "Stop sharing →", "command": "stop"}));
        } else {
            lines.push(json!({"text": "Leave →", "command": "stop"}));
        }
        notify_folio("status.set", json!({"text": text}));
        notify_folio("panel.set", json!({"id": "people", "lines": lines}));
    }

    // ---- document sync ----

    fn on_snapshot(&mut self, params: &Value) {
        let snap = Snapshot {
            revision: params["revision"].as_u64().unwrap_or(0),
            paragraphs: serde_json::from_value(params["paragraphs"].clone()).unwrap_or_default(),
            cursor: pos(&params["cursor"]),
            anchor: pos(&params["anchor"]),
        };
        self.snapshot = Some(snap.clone());
        let Some(s) = &mut self.session else { return };
        let Some(crdt) = &mut s.crdt else { return };
        if !s.on_screen {
            // A guest whose window doesn't show the shared document yet.
            self.merge_if_behind();
            return;
        }
        if s.revision != Some(snap.revision) {
            crdt.apply_local(&snap.paragraphs);
            s.revision = Some(snap.revision);
        }
        if !crdt.folio_behind() {
            let caret = crdt.cursor_at(snap.cursor).zip(crdt.cursor_at(snap.anchor));
            s.caret = caret;
        }
        self.broadcast_updates();
        self.broadcast_my_presence();
        self.merge_if_behind();
        self.push_presence();
    }

    fn undo_redo(&mut self, undo: bool) {
        let Some(s) = &mut self.session else { return };
        let Some(crdt) = &mut s.crdt else { return };
        let before = crdt.paragraphs();
        let changed = if undo { crdt.undo() } else { crdt.redo() };
        if !changed {
            return;
        }
        let at = crdt.change_end(&before);
        s.caret = crdt.cursor_at(at).zip(crdt.cursor_at(at));
        self.broadcast_updates();
        self.merge_if_behind();
    }

    /// Sends the CRDT's state to Folio if Folio is missing changes.
    fn merge_if_behind(&mut self) {
        let Some(s) = &mut self.session else { return };
        let Some(crdt) = &s.crdt else { return };
        if s.merging || (s.on_screen && !crdt.folio_behind()) {
            return;
        }
        let Some(base) = s.revision.or(self.snapshot.as_ref().map(|x| x.revision)) else {
            return;
        };
        let (cursor, anchor) = match &s.caret {
            Some((c, a)) => (crdt.resolve(c), crdt.resolve(a)),
            None => (None, None),
        };
        let fallback = Pos::default();
        let params = json!({
            "base_revision": base,
            "paragraphs": crdt.paragraphs(),
            "cursor": pos_json(cursor.unwrap_or(fallback)),
            "anchor": pos_json(anchor.or(cursor).unwrap_or(fallback)),
        });
        let frontier = crdt.frontier();
        s.merging = true;
        self.request("doc.merge", params, Pending::Merge { frontier, base });
    }

    /// Hands local changes to the network.
    fn broadcast_updates(&mut self) {
        let Some(s) = &mut self.session else { return };
        let Some(crdt) = &mut s.crdt else { return };
        let Some(bytes) = crdt.take_updates() else {
            return;
        };
        match &s.role {
            Role::Host { guests, .. } => {
                for g in guests.values().filter(|g| g.admitted) {
                    let _ = g.tx.send(Frame::Update(bytes.clone()));
                }
            }
            Role::Guest { host, .. } => {
                let _ = host.send(Frame::Update(bytes));
            }
        }
    }

    fn my_presence(&self) -> Option<Presence> {
        let s = self.session.as_ref()?;
        let who = net::short_id(&s.endpoint.id());
        Some(Presence {
            color: color_for(&who),
            who,
            name: self.name.clone(),
            cursor: s.caret.as_ref().map(|c| c.0.encode()),
            anchor: s.caret.as_ref().map(|c| c.1.encode()),
        })
    }

    fn broadcast_my_presence(&mut self) {
        let Some(me) = self.my_presence() else { return };
        let Some(s) = &self.session else { return };
        match &s.role {
            Role::Host { guests, .. } => {
                for g in guests.values().filter(|g| g.admitted) {
                    let _ = g.tx.send(Frame::Presence(me.clone()));
                }
            }
            Role::Guest { host, .. } => {
                let _ = host.send(Frame::Presence(me));
            }
        }
    }

    /// Shows other people's carets in Folio, if they moved.
    fn push_presence(&mut self) {
        let Some(s) = &mut self.session else { return };
        let Some(crdt) = &s.crdt else { return };
        if !s.on_screen || crdt.folio_behind() {
            return;
        }
        let peers: Vec<Value> = s
            .peers
            .values()
            .filter_map(|p| {
                let cursor = crdt.resolve(&Cursor::decode(p.cursor.as_ref()?).ok()?)?;
                let anchor = p
                    .anchor
                    .as_ref()
                    .and_then(|a| Cursor::decode(a).ok())
                    .and_then(|a| crdt.resolve(&a))
                    .unwrap_or(cursor);
                Some(json!({"name": p.name, "color": p.color,
                    "cursor": pos_json(cursor), "anchor": pos_json(anchor)}))
            })
            .collect();
        let value = json!({"peers": peers});
        if value != s.sent_presence {
            s.sent_presence = value.clone();
            notify_folio("presence.set", value);
        }
    }

    // ---- network ----

    fn net_sender(&self) -> UnboundedSender<NetEvent> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let inputs = self.inputs.clone();
        tokio::spawn(async move {
            while let Some(e) = rx.recv().await {
                if inputs.send(Input::Net(e)).is_err() {
                    break;
                }
            }
        });
        tx
    }

    fn on_net(&mut self, event: NetEvent) {
        let Some(s) = &mut self.session else { return };
        match (&mut s.role, event) {
            (Role::Host { guests, .. }, NetEvent::Opened { peer, who, tx }) => {
                guests.insert(
                    peer,
                    Guest {
                        tx,
                        who,
                        name: String::new(),
                        admitted: false,
                    },
                );
            }
            (Role::Host { .. }, NetEvent::Frame { peer, frame }) => self.host_frame(peer, frame),
            (Role::Host { guests, .. }, NetEvent::Closed { peer }) => {
                let Some(g) = guests.remove(&peer) else {
                    return;
                };
                if !g.admitted {
                    return;
                }
                for other in guests.values().filter(|o| o.admitted) {
                    let _ = other.tx.send(Frame::Leave { who: g.who.clone() });
                }
                s.peers.remove(&g.who);
                let name = g.name;
                self.notify(&format!("{name} left."));
                self.status();
                self.push_presence();
            }
            (Role::Guest { .. }, NetEvent::Frame { frame, .. }) => self.guest_frame(frame),
            (Role::Guest { host_name, .. }, NetEvent::Closed { .. }) => {
                let text = match host_name {
                    Some(name) => format!("{name} stopped sharing."),
                    None => "The host didn't let you in.".into(),
                };
                self.stop(false);
                self.notify(&text);
            }
            (Role::Guest { .. }, NetEvent::Opened { .. }) => {}
        }
    }

    fn host_frame(&mut self, peer: u64, frame: Frame) {
        let Some(s) = &mut self.session else { return };
        let Role::Host {
            guests,
            doc_id,
            secret,
            ..
        } = &mut s.role
        else {
            return;
        };
        let Some(g) = guests.get_mut(&peer) else {
            return;
        };
        match frame {
            Frame::Hello {
                doc,
                secret: given,
                name,
            } if !g.admitted && g.name.is_empty() => {
                if doc != *doc_id || given != *secret {
                    let _ = g.tx.send(Frame::Denied {
                        reason: "This invite is no longer valid.".into(),
                    });
                    return;
                }
                let name: String = name.chars().filter(|c| !c.is_control()).take(60).collect();
                g.name = if name.is_empty() {
                    "Someone".into()
                } else {
                    name
                };
                let name = g.name.clone();
                self.request(
                    "ui.confirm",
                    json!({
                        "title": format!("Let {name} join?"),
                        "message": format!("{name} wants to edit this document with you. They'll see all of it and can change it."),
                    }),
                    Pending::Admit(peer),
                );
            }
            Frame::Update(bytes) if g.admitted => {
                let Some(crdt) = &mut s.crdt else { return };
                // Local changes always go out first.
                if let Some(local) = crdt.take_updates() {
                    for o in guests.values().filter(|o| o.admitted) {
                        let _ = o.tx.send(Frame::Update(local.clone()));
                    }
                }
                if crdt.import(&bytes).is_err() {
                    return;
                }
                for (_, o) in guests.iter().filter(|(id, o)| **id != peer && o.admitted) {
                    let _ = o.tx.send(Frame::Update(bytes.clone()));
                }
                self.merge_if_behind();
                self.push_presence();
            }
            Frame::Presence(mut p) if g.admitted => {
                p.who = g.who.clone();
                p.name = g.name.clone();
                p.color = color_for(&p.who);
                for (_, o) in guests.iter().filter(|(id, o)| **id != peer && o.admitted) {
                    let _ = o.tx.send(Frame::Presence(p.clone()));
                }
                s.peers.insert(p.who.clone(), p);
                self.push_presence();
            }
            _ => {}
        }
    }

    fn admit(&mut self, peer: u64, yes: bool) {
        let me = self.my_presence();
        let host_name = self.name.clone();
        let Some(s) = &mut self.session else { return };
        let Role::Host { guests, .. } = &mut s.role else {
            return;
        };
        let Some(crdt) = &mut s.crdt else { return };
        if !guests.contains_key(&peer) {
            return;
        }
        if !yes {
            if let Some(g) = guests.get(&peer) {
                let _ = g.tx.send(Frame::Denied {
                    reason: "The host said no.".into(),
                });
            }
            return;
        }
        // Everyone else gets our pending changes before the snapshot is cut.
        if let Some(local) = crdt.take_updates() {
            for o in guests.values().filter(|o| o.admitted) {
                let _ = o.tx.send(Frame::Update(local.clone()));
            }
        }
        let snapshot = crdt.export_snapshot();
        let g = guests.get_mut(&peer).expect("checked above");
        g.admitted = true;
        let _ = g.tx.send(Frame::Welcome { name: host_name });
        let _ = g.tx.send(Frame::Update(snapshot));
        if let Some(me) = me {
            let _ = g.tx.send(Frame::Presence(me));
        }
        for p in s.peers.values() {
            let _ = g.tx.send(Frame::Presence(p.clone()));
        }
        let name = g.name.clone();
        self.notify(&format!("{name} joined."));
        self.status();
    }

    fn guest_frame(&mut self, frame: Frame) {
        let Some(s) = &mut self.session else { return };
        let Role::Guest { host_name, .. } = &mut s.role else {
            return;
        };
        match frame {
            Frame::Welcome { name } => *host_name = Some(name),
            Frame::Denied { reason } => {
                self.stop(false);
                self.notify(&format!("Couldn't join: {reason}"));
            }
            Frame::Update(bytes) if host_name.is_some() => {
                match &mut s.crdt {
                    None => match Crdt::from_snapshot(&bytes) {
                        Ok(crdt) => {
                            s.crdt = Some(crdt);
                            self.set_session(true);
                            self.status();
                        }
                        Err(e) => {
                            self.stop(false);
                            self.notify(&format!("Couldn't read the shared document: {e}"));
                            return;
                        }
                    },
                    Some(crdt) => {
                        let local = crdt.take_updates();
                        if crdt.import(&bytes).is_err() {
                            return;
                        }
                        if let (Some(local), Role::Guest { host, .. }) = (local, &s.role) {
                            let _ = host.send(Frame::Update(local));
                        }
                    }
                }
                self.merge_if_behind();
                self.push_presence();
            }
            Frame::Presence(p) => {
                s.peers.insert(p.who.clone(), p);
                self.status();
                self.push_presence();
            }
            Frame::Leave { who } => {
                if let Some(p) = s.peers.remove(&who) {
                    self.notify(&format!("{} left.", p.name));
                }
                self.status();
                self.push_presence();
            }
            _ => {}
        }
    }
}

/// Runs a short task to completion from inside the runtime's thread.
fn futures_block(handle: tokio::task::JoinHandle<()>) -> Option<()> {
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(2), handle)
                .await
                .ok()?
                .ok()
        })
    })
}

#[tokio::main]
async fn main() {
    let (inputs, mut rx) = mpsc::unbounded_channel();
    {
        let inputs = inputs.clone();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str(&line) {
                    Ok(v) => {
                        let _ = inputs.send(Input::Folio(v));
                    }
                    Err(e) => eprintln!("folio-link: bad message: {e}"),
                }
            }
            let _ = inputs.send(Input::Eof);
        });
    }
    let settings = net::load_settings();
    let mut app = App {
        next_id: 0,
        pending: HashMap::new(),
        name: net::display_name(&settings),
        settings,
        snapshot: None,
        session: None,
        session_seed: None,
        join_invite: None,
        starting: false,
        inputs,
    };
    while let Some(input) = rx.recv().await {
        match input {
            Input::Folio(v) => app.handle_folio(v),
            Input::Net(e) => app.on_net(e),
            Input::Hosted(r) => app.on_hosted(r),
            Input::Joined(r) => app.on_joined(r),
            Input::Eof => app.quit(),
        }
    }
}
