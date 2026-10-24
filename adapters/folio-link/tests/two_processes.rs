//! Two `folio-link` processes on this machine (relay "none") share a
//! document, each driven only through the adapter protocol by a fake Folio.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Enough of Folio to host an adapter: answers requests, applies merges
/// when their base revision matches, and sends a snapshot on every change.
struct FakeFolio {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<Value>,
    revision: u64,
    paragraphs: Value,
    cursor: Value,
    status: Option<String>,
    notices: Vec<String>,
    presence: Value,
    session_active: bool,
}

impl FakeFolio {
    fn spawn(dir: &Path, clipboard: &Path, name: &str, paragraphs: Value) -> Self {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("settings.toml"),
            format!("relay = \"none\"\nname = \"{name}\"\n"),
        )
        .unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_folio-link"))
            .env("FOLIO_LINK_DIR", dir)
            .env("FOLIO_LINK_CLIPBOARD", clipboard)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let v: Value = serde_json::from_str(&line).expect("adapter sent bad JSON");
                if tx.send(v).is_err() {
                    break;
                }
            }
        });
        let stdin = child.stdin.take().unwrap();
        let mut folio = FakeFolio {
            child,
            stdin,
            lines,
            revision: 1,
            paragraphs,
            cursor: json!({"para": 0, "ch": 0}),
            status: None,
            notices: Vec::new(),
            presence: json!([]),
            session_active: false,
        };
        folio.send(json!({"method": "initialize", "params": {
            "folio": "0.3.0", "protocol": 3,
            "permissions": ["document.read", "document.edit", "network"]}}));
        folio.snapshot();
        folio
    }

    fn send(&mut self, v: Value) {
        writeln!(self.stdin, "{v}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn command(&mut self, command: &str) {
        self.send(json!({"method": "command", "params": {"command": command}}));
    }

    fn snapshot(&mut self) {
        let params = json!({"kind": "snapshot", "revision": self.revision,
            "paragraphs": self.paragraphs, "cursor": self.cursor, "anchor": self.cursor});
        self.send(json!({"method": "event", "params": params}));
    }

    /// The user types: a new revision and a snapshot.
    fn edit(&mut self, paragraphs: Value, cursor: Value) {
        self.revision += 1;
        self.paragraphs = paragraphs;
        self.cursor = cursor;
        self.snapshot();
    }

    /// Handles everything the adapter has sent so far.
    fn pump(&mut self) {
        while let Ok(msg) = self.lines.recv_timeout(Duration::from_millis(5)) {
            let params = &msg["params"];
            let result = match msg["method"].as_str().unwrap_or("") {
                "doc.get" => json!({"paragraphs": self.paragraphs, "path": null, "theme": "Paper"}),
                "ui.confirm" => json!(true),
                "doc.merge" => {
                    if params["base_revision"].as_u64() == Some(self.revision) {
                        self.revision += 1;
                        self.paragraphs = params["paragraphs"].clone();
                        self.cursor = params["cursor"].clone();
                        let reply = json!({"id": msg["id"], "result": {"applied": true}});
                        self.send(reply);
                        self.snapshot();
                        continue;
                    }
                    json!({"applied": false})
                }
                "session.set" => {
                    self.session_active = params["active"] == true;
                    Value::Null
                }
                "status.set" => {
                    self.status = params["text"].as_str().map(String::from);
                    Value::Null
                }
                "notify" => {
                    self.notices
                        .push(params["text"].as_str().unwrap_or("").into());
                    Value::Null
                }
                "presence.set" => {
                    self.presence = params["peers"].clone();
                    Value::Null
                }
                _ => Value::Null,
            };
            if msg.get("id").is_some() {
                self.send(json!({"id": msg["id"], "result": result}));
            }
        }
    }
}

fn until(
    a: &mut FakeFolio,
    b: &mut FakeFolio,
    what: &str,
    done: impl Fn(&FakeFolio, &FakeFolio) -> bool,
) {
    let start = Instant::now();
    while !done(a, b) {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "timed out waiting for {what}\nhost: {:?} {} {:?}\nguest: {:?} {} {:?}",
            a.status,
            a.paragraphs,
            a.notices,
            b.status,
            b.paragraphs,
            b.notices
        );
        a.pump();
        b.pump();
    }
}

fn body(text: &str) -> Value {
    json!({"style": "body", "runs": [{"text": text}]})
}

#[test]
fn two_processes_sync_a_document() {
    let dir: PathBuf =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let clipboard = dir.join("clipboard");
    let start = json!([
        {"style": "h1", "runs": [{"text": "Plan"}]},
        body("Hello"),
    ]);
    let mut host = FakeFolio::spawn(&dir.join("host"), &clipboard, "Hana", start.clone());
    let mut guest = FakeFolio::spawn(
        &dir.join("guest"),
        &clipboard,
        "Gus",
        json!([{"style": "body", "runs": []}]),
    );

    host.command("share");
    until(&mut host, &mut guest, "the invite", |h, _| {
        h.notices.iter().any(|n| n.starts_with("Invite copied"))
    });
    assert_eq!(host.status.as_deref(), Some("● Sharing · 1 person"));
    assert!(std::fs::read_to_string(&clipboard)
        .unwrap()
        .starts_with("folio-link:"));

    guest.command("join");
    until(&mut host, &mut guest, "the join", |h, g| {
        g.paragraphs == start
            && g.status.as_deref() == Some("● Joined Hana's document")
            && h.status.as_deref() == Some("● Sharing · 2 people")
    });
    assert!(guest.session_active);

    // The guest types a bold word at the end.
    let typed = json!([
        {"style": "h1", "runs": [{"text": "Plan"}]},
        {"style": "body", "runs": [{"text": "Hello "}, {"text": "world", "bold": true}]},
    ]);
    guest.edit(typed.clone(), json!({"para": 1, "ch": 11}));
    until(&mut host, &mut guest, "the guest's edit", |h, _| {
        h.paragraphs == typed
    });

    // Let the guest's undo history close that step.
    std::thread::sleep(Duration::from_millis(700));

    // Both type at once: the host restyles the title, the guest adds a line.
    host.edit(
        json!([{"style": "h2", "runs": [{"text": "Plan"}]}, typed[1]]),
        json!({"para": 0, "ch": 4}),
    );
    guest.edit(
        json!([typed[0], typed[1], body("More")]),
        json!({"para": 2, "ch": 4}),
    );
    let both = json!([{"style": "h2", "runs": [{"text": "Plan"}]}, typed[1], body("More")]);
    until(&mut host, &mut guest, "convergence", |h, g| {
        h.paragraphs == both && g.paragraphs == both
    });

    // Each sees the other's caret.
    until(&mut host, &mut guest, "presence", |h, g| {
        h.presence.as_array().is_some_and(|p| {
            p.iter()
                .any(|x| x["name"] == "Gus" && x["cursor"] == json!({"para": 2, "ch": 4}))
        }) && g
            .presence
            .as_array()
            .is_some_and(|p| p.iter().any(|x| x["name"] == "Hana"))
    });

    // Undo on the guest takes back only the guest's own last edit.
    guest.send(json!({"method": "undo", "params": {}}));
    let undone = json!([{"style": "h2", "runs": [{"text": "Plan"}]}, typed[1]]);
    until(&mut host, &mut guest, "the undo", |h, g| {
        h.paragraphs == undone && g.paragraphs == undone
    });

    // The guest leaves; the host is alone again.
    guest.command("stop");
    until(&mut host, &mut guest, "the leave", |h, g| {
        h.status.as_deref() == Some("● Sharing · 1 person")
            && g.status.is_none()
            && !g.session_active
    });

    for f in [&mut host, &mut guest] {
        f.send(json!({"method": "shutdown"}));
    }
    for f in [&mut host, &mut guest] {
        let start = Instant::now();
        while f.child.try_wait().unwrap().is_none() {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "adapter didn't exit"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
