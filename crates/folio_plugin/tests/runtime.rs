//! End-to-end tests: a real WebAssembly plugin (`tests/guest`) running in
//! the real runtime, probing what the sandbox allows.
//!
//! Needs the WebAssembly target: `rustup target add wasm32-wasip2`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use folio_plugin::{
    DocumentSnapshot, Edit, Event, Limits, PluginError, PluginOutput, PluginRuntime, Position, RuntimeConfig, Selection,
};

/// Builds the probe plugin once for all tests.
fn probe_wasm() -> &'static Path {
    static WASM: OnceLock<PathBuf> = OnceLock::new();
    WASM.get_or_init(|| {
        let guest = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/guest");
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "--release", "--target", "wasm32-wasip2", "--manifest-path"])
            .arg(guest.join("Cargo.toml"))
            .status()
            .expect("cargo runs");
        assert!(status.success(), "building the probe plugin failed (is the wasm32-wasip2 target installed?)");
        guest.join("target/wasm32-wasip2/release/probe_plugin.wasm")
    })
}

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("folio-plugin-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("plugins")).unwrap();
        Self { dir }
    }

    /// Installs the probe plugin under `name` with extra manifest lines.
    fn install(&self, name: &str, extra: &str) {
        let folder = self.dir.join("plugins").join(name);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::copy(probe_wasm(), folder.join("probe.wasm")).unwrap();
        std::fs::write(
            folder.join("plugin.toml"),
            format!("name = \"{name}\"\nversion = \"1.0.0\"\nwasm = \"probe.wasm\"\n{extra}"),
        )
        .unwrap();
    }

    fn start(&self, limits: Limits) -> Runtime {
        let rt = PluginRuntime::start(RuntimeConfig {
            plugins_dir: self.dir.join("plugins"),
            limits,
            waker: None,
        })
        .unwrap();
        Runtime {
            rt,
            seen: Vec::new(),
            used: Vec::new(),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Runtime {
    rt: PluginRuntime,
    /// Every output so far, oldest first.
    seen: Vec<PluginOutput>,
    /// Which of them a `wait_for` has already returned.
    used: Vec<bool>,
}

impl Runtime {
    /// Polls until an output matches, and returns the oldest such output
    /// not returned before.
    fn wait_for(&mut self, what: &str, pred: impl Fn(&PluginOutput) -> bool) -> PluginOutput {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            self.seen.extend(self.rt.poll());
            self.used.resize(self.seen.len(), false);
            if let Some(i) = (0..self.seen.len()).find(|&i| !self.used[i] && pred(&self.seen[i])) {
                self.used[i] = true;
                return self.seen[i].clone();
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}; got {:#?}", self.seen);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Waits for a plugin's notification and returns its text.
    fn notice(&mut self, plugin: &str) -> String {
        match self.wait_for(&format!("a notice from {plugin}"), |o| matches!(o, PluginOutput::Notify { plugin: p, .. } if p == plugin)) {
            PluginOutput::Notify { message, .. } => message,
            _ => unreachable!(),
        }
    }

    fn loaded(&mut self, names: &[&str]) {
        for name in names {
            self.wait_for(&format!("{name} to load"), |o| matches!(o, PluginOutput::Loaded(i) if i.name == *name));
        }
    }

    /// Runs a command and returns the notice it produces.
    fn ask(&mut self, plugin: &str, command: &str) -> String {
        self.rt.run_command(plugin, command);
        self.notice(plugin)
    }

    fn error(&mut self, what: &str, pred: impl Fn(&PluginError) -> bool) -> PluginError {
        match self.wait_for(what, |o| matches!(o, PluginOutput::Error(e) if pred(e))) {
            PluginOutput::Error(e) => e,
            _ => unreachable!(),
        }
    }
}

fn essay() -> DocumentSnapshot {
    DocumentSnapshot {
        revision: 7,
        title: "Essay".into(),
        path: None,
        theme: "Paper".into(),
        paragraphs: 2,
        words: 3,
        characters: 17,
        selection: Some(Selection {
            start: Position { paragraph: 1, character: 4 },
            end: Position { paragraph: 1, character: 9 },
            text: "world".into(),
        }),
    }
}

#[test]
fn document_access_follows_capabilities() {
    let f = Fixture::new("document");
    f.install("Nothing", "");
    f.install("Reader", "[capabilities]\ndocument = [\"read\"]");
    f.install("Editor", "[capabilities]\ndocument = [\"read\", \"edit\"]");
    let mut rt = f.start(Limits::default());
    rt.loaded(&["Nothing", "Reader", "Editor"]);
    rt.rt.set_document(essay());

    rt.rt.run_command("Nothing", "hello");
    let e = rt.error("Nothing to be refused", |e| matches!(e, PluginError::Guest { plugin, .. } if plugin == "Nothing"));
    assert!(e.to_string().contains("missing-capability"), "{e}");
    assert_eq!(rt.ask("Nothing", "selection"), "selection denied: missing-capability");

    assert_eq!(rt.ask("Reader", "hello"), "Essay has 3 words");
    assert_eq!(rt.notice("Reader"), "insert denied: missing-capability");
    assert_eq!(rt.ask("Reader", "selection"), "selected \"world\" from 1:4");

    assert_eq!(rt.ask("Editor", "hello"), "Essay has 3 words");
    assert_eq!(rt.notice("Editor"), "inserted");
    let edit = rt.wait_for("the edit", |o| matches!(o, PluginOutput::Edit { .. }));
    let PluginOutput::Edit { plugin, revision, edit } = edit else { unreachable!() };
    assert_eq!((plugin.as_str(), revision), ("Editor", 7), "edits carry the revision they were made against");
    assert_eq!(edit, Edit::InsertText("Hi from wasm".into()));
}

#[test]
fn filesystem_and_environment_are_denied_unless_granted() {
    let f = Fixture::new("files");
    let notes = f.dir.join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    std::fs::write(notes.join("note.txt"), "hello from the host").unwrap();
    f.install("Blind", "");
    f.install(
        "Sighted",
        &format!("[capabilities]\nfilesystem = [{{ path = {:?}, mount = \"/notes\" }}]", notes.display().to_string()),
    );
    f.install(
        "Writer",
        &format!(
            "[capabilities]\nfilesystem = [{{ path = {:?}, access = \"read-write\", mount = \"/notes\" }}]",
            notes.display().to_string()
        ),
    );
    let mut rt = f.start(Limits::default());
    rt.loaded(&["Blind", "Sighted", "Writer"]);

    assert!(rt.ask("Blind", "read:/notes/note.txt").starts_with("read failed"));
    assert!(rt.ask("Blind", "read:/etc/hosts").starts_with("read failed"));
    assert_eq!(rt.ask("Blind", "env"), "0 environment variables");

    assert_eq!(rt.ask("Sighted", "read:/notes/note.txt"), "read ok: hello from the host");
    assert!(rt.ask("Sighted", "read:/etc/hosts").starts_with("read failed"), "only the granted folder");
    assert!(rt.ask("Sighted", "read:/notes/../../etc/hosts").starts_with("read failed"), "no escaping the folder");
    assert!(rt.ask("Sighted", "write:/notes/new.txt").starts_with("write failed"), "read access can't write");
    assert!(!notes.join("new.txt").exists());

    assert_eq!(rt.ask("Writer", "write:/notes/new.txt"), "write ok");
    assert_eq!(std::fs::read_to_string(notes.join("new.txt")).unwrap(), "written by a plugin");
}

#[test]
fn network_is_denied_unless_granted() {
    let allowed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let (allowed, other) = (allowed.local_addr().unwrap(), other.local_addr().unwrap());
    let f = Fixture::new("network");
    f.install("Offline", "");
    f.install("Online", &format!("[capabilities]\nnetwork = [\"{allowed}\"]"));
    f.install("Anywhere", "[capabilities]\nnetwork = \"any\"");
    let mut rt = f.start(Limits::default());
    rt.loaded(&["Offline", "Online", "Anywhere"]);

    assert!(rt.ask("Offline", &format!("connect:{allowed}")).starts_with("connect failed"));
    assert_eq!(rt.ask("Online", &format!("connect:{allowed}")), "connected");
    assert!(
        rt.ask("Online", &format!("connect:{other}")).starts_with("connect failed"),
        "only the listed address"
    );
    assert_eq!(rt.ask("Anywhere", &format!("connect:{other}")), "connected");
}

#[test]
fn runaway_plugins_are_stopped_without_blocking_the_editor() {
    let f = Fixture::new("runaway");
    f.install("Probe", "");
    let limits = Limits {
        event_timeout: Duration::from_millis(300),
        ..Limits::default()
    };
    let mut rt = f.start(limits);
    rt.loaded(&["Probe"]);

    assert_eq!(rt.ask("Probe", "count"), "count 1");
    assert_eq!(rt.ask("Probe", "count"), "count 2", "a plugin keeps its state between events");

    rt.rt.run_command("Probe", "spin");
    // The editor's side never waits on a plugin.
    let started = Instant::now();
    for _ in 0..100 {
        rt.rt.set_document(essay());
        rt.rt.dispatch(Event::Changed);
        rt.seen.extend(rt.rt.poll());
    }
    assert!(started.elapsed() < Duration::from_millis(100), "{:?}", started.elapsed());

    let e = rt.error("the timeout", |e| matches!(e, PluginError::Timeout { .. }));
    assert!(e.to_string().contains("300ms"), "{e}");
    assert_eq!(rt.ask("Probe", "count"), "count 1", "restarted with a fresh instance");

    // A plugin's own error isn't a crash: it keeps its state.
    rt.rt.run_command("Probe", "fail");
    rt.error("the plugin's error", |e| matches!(e, PluginError::Guest { message, .. } if message.contains("always fails")));
    assert_eq!(rt.ask("Probe", "count"), "count 2");

    rt.rt.run_command("Probe", "trap");
    rt.error("the trap", |e| matches!(e, PluginError::Trap { .. }));
    assert_eq!(rt.ask("Probe", "count"), "count 1");
}

#[test]
fn plugins_that_keep_failing_are_turned_off() {
    let f = Fixture::new("disabled");
    f.install("Probe", "");
    let limits = Limits {
        max_failures: 2,
        ..Limits::default()
    };
    let mut rt = f.start(limits);
    rt.loaded(&["Probe"]);
    rt.rt.run_command("Probe", "trap");
    rt.rt.run_command("Probe", "trap");
    let e = rt.error("the plugin to be turned off", |e| matches!(e, PluginError::Disabled { .. }));
    assert!(e.to_string().contains("2 times"), "{e}");
    // Later commands go nowhere instead of failing the editor.
    rt.rt.run_command("Probe", "count");
    std::thread::sleep(Duration::from_millis(200));
    rt.seen.extend(rt.rt.poll());
    assert!(!rt.seen.iter().any(|o| matches!(o, PluginOutput::Notify { .. })));
}

#[test]
fn budgets_and_memory_are_limited() {
    let f = Fixture::new("limits");
    f.install("Probe", "[capabilities]\ndocument = [\"read\", \"edit\"]");
    let limits = Limits {
        messages_per_event: 50,
        memory_bytes: 32 << 20,
        ..Limits::default()
    };
    let mut rt = f.start(limits);
    rt.loaded(&["Probe"]);
    rt.rt.set_document(essay());

    rt.rt.run_command("Probe", "spam");
    rt.wait_for("the plugin to give up", |o| matches!(o, PluginOutput::Edit { .. }));
    std::thread::sleep(Duration::from_millis(200));
    rt.seen.extend(rt.rt.poll());
    let edits = rt.seen.iter().filter(|o| matches!(o, PluginOutput::Edit { .. })).count();
    assert_eq!(edits, 50, "at most messages_per_event per event");

    rt.rt.run_command("Probe", "grow");
    let e = rt.error("running out of memory", |e| matches!(e, PluginError::Trap { .. }));
    assert!(!e.to_string().is_empty());
    assert_eq!(rt.ask("Probe", "count"), "count 1", "and it recovers");
}

#[test]
fn events_go_to_subscribers_with_the_current_document() {
    let f = Fixture::new("events");
    f.install("Listener", "events = [\"changed\", \"saved\"]\n[capabilities]\ndocument = [\"read\"]");
    f.install("Deaf", "[capabilities]\ndocument = [\"read\"]");
    let mut rt = f.start(Limits::default());
    rt.loaded(&["Listener", "Deaf"]);

    rt.rt.set_document(DocumentSnapshot { revision: 41, ..essay() });
    rt.rt.dispatch(Event::Changed);
    assert_eq!(rt.notice("Listener"), "changed to revision 41");
    rt.rt.dispatch(Event::Saved("/tmp/essay.folio".into()));
    assert_eq!(rt.notice("Listener"), "saved /tmp/essay.folio");
    assert!(
        !rt.seen.iter().any(|o| matches!(o, PluginOutput::Notify { plugin, .. } if plugin == "Deaf")),
        "only subscribers get events"
    );

    rt.rt.run_command("Nobody", "hello");
    rt.error("an unknown plugin", |e| matches!(e, PluginError::NotLoaded(n) if n == "Nobody"));
}

#[test]
fn bad_plugins_are_reported_and_skipped() {
    let f = Fixture::new("bad");
    f.install("Good", "");
    let broken = f.dir.join("plugins/broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("plugin.toml"), "name = \"Broken\"\nversion = \"1\"").unwrap();
    std::fs::write(broken.join("plugin.wasm"), b"not webassembly").unwrap();
    let mut rt = f.start(Limits::default());
    rt.loaded(&["Good"]);
    rt.error("the broken plugin", |e| matches!(e, PluginError::Load { plugin, .. } if plugin == "Broken"));
    assert_eq!(rt.ask("Good", "count"), "count 1");
    rt.rt.shutdown();
}
