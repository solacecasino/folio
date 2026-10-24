//! A plugin that probes the sandbox. Each command tries one thing and
//! reports the outcome with `notify`, so the host tests can check what a
//! plugin can and can't do.

use std::sync::atomic::{AtomicU32, Ordering};

wit_bindgen::generate!({
    path: "../../wit",
    world: "plugin",
});

use exports::folio::plugin::guest::{Event, Guest};
use folio::plugin::{document, host};

struct Probe;

static COUNT: AtomicU32 = AtomicU32::new(0);

/// A refusal's WIT name, such as "missing-capability".
fn why(denied: document::Denied) -> &'static str {
    match denied {
        document::Denied::MissingCapability => "missing-capability",
        document::Denied::NoDocument => "no-document",
        document::Denied::LimitReached => "limit-reached",
    }
}

fn say(message: impl AsRef<str>) {
    host::notify(message.as_ref());
}

impl Guest for Probe {
    fn on_event(event: Event) -> Result<(), String> {
        match event {
            Event::Startup => host::log(host::Level::Info, "probe started"),
            Event::Changed => match document::get_metadata() {
                Ok(m) => say(format!("changed to revision {}", m.revision)),
                Err(e) => say(format!("metadata denied: {}", why(e))),
            },
            Event::Saved(path) => say(format!("saved {path}")),
            Event::SelectionChanged => {}
            Event::Command(command) => run(&command)?,
        }
        Ok(())
    }
}

fn run(command: &str) -> Result<(), String> {
    let (name, arg) = command.split_once(':').unwrap_or((command, ""));
    match name {
        "hello" => {
            let m = document::get_metadata().map_err(|e| format!("metadata denied: {}", why(e)))?;
            say(format!("{} has {} words", m.title, m.words));
            match document::insert_text("Hi from wasm") {
                Ok(()) => say("inserted"),
                Err(e) => say(format!("insert denied: {}", why(e))),
            }
        }
        "selection" => match document::get_selection() {
            Ok(Some(s)) => say(format!("selected \"{}\" from {}:{}", s.text, s.start.paragraph, s.start.character)),
            Ok(None) => say("just a caret"),
            Err(e) => say(format!("selection denied: {}", why(e))),
        },
        "read" => match std::fs::read_to_string(arg) {
            Ok(text) => say(format!("read ok: {}", text.trim())),
            Err(e) => say(format!("read failed: {e}")),
        },
        "write" => match std::fs::write(arg, "written by a plugin") {
            Ok(()) => say("write ok"),
            Err(e) => say(format!("write failed: {e}")),
        },
        "connect" => match std::net::TcpStream::connect(arg) {
            Ok(_) => say("connected"),
            Err(e) => say(format!("connect failed: {e}")),
        },
        "env" => say(format!("{} environment variables", std::env::vars().count())),
        "count" => say(format!("count {}", COUNT.fetch_add(1, Ordering::SeqCst) + 1)),
        "spam" => {
            let mut sent = 0;
            loop {
                match document::insert_text("x") {
                    Ok(()) => sent += 1,
                    Err(e) => {
                        host::log(host::Level::Warn, &format!("stopped after {sent}: {}", why(e)));
                        break;
                    }
                }
            }
        }
        "spin" => loop {
            std::hint::black_box(COUNT.load(Ordering::Relaxed));
        },
        "grow" => {
            // Far past the memory limit.
            let mut blocks: Vec<Vec<u8>> = Vec::new();
            loop {
                blocks.push(vec![1u8; 16 << 20]);
            }
        }
        "fail" => return Err("this command always fails".into()),
        "trap" => panic!("probe trapped on purpose"),
        other => return Err(format!("unknown command {other}")),
    }
    Ok(())
}

export!(Probe);
