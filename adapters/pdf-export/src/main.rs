//! PDF Export: a Folio adapter. Folio and an adapter exchange one JSON object
//! per line over stdin/stdout; see docs/adapters.md for the protocol.
//!
//! On the `export` command it asks Folio for the print model, asks where to
//! save, writes the PDF and reports back with `notify`.

mod pdf;

use std::collections::VecDeque;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// Folio quit or closed the pipe; stop quietly.
struct Gone;

struct Conn<R, W> {
    input: R,
    output: W,
    next_id: u64,
    /// Messages that arrived while waiting for a reply.
    backlog: VecDeque<Value>,
}

impl<R: BufRead, W: Write> Conn<R, W> {
    fn send(&mut self, message: Value) -> Result<(), Gone> {
        writeln!(self.output, "{message}").and_then(|()| self.output.flush()).map_err(|_| Gone)
    }

    fn read(&mut self) -> Result<Value, Gone> {
        loop {
            let mut line = String::new();
            if self.input.read_line(&mut line).map_err(|_| Gone)? == 0 {
                return Err(Gone);
            }
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str(&line) {
                Ok(message) => return Ok(message),
                Err(e) => self.log(&format!("ignoring a line that isn't JSON: {e}"))?,
            }
        }
    }

    fn next(&mut self) -> Result<Value, Gone> {
        match self.backlog.pop_front() {
            Some(message) => Ok(message),
            None => self.read(),
        }
    }

    /// Sends a request and waits for its reply. The outer `Result` is the pipe,
    /// the inner one is Folio's answer.
    fn request(&mut self, method: &str, params: Value) -> Result<Result<Value, String>, Gone> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"id": id, "method": method, "params": params}))?;
        loop {
            let message = self.read()?;
            if message.get("id").and_then(Value::as_u64) == Some(id) && message.get("method").is_none() {
                return Ok(match message.get("error") {
                    Some(e) => Err(e.as_str().map_or_else(|| e.to_string(), str::to_owned)),
                    None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                });
            }
            if message.get("method").and_then(Value::as_str) == Some("shutdown") {
                return Err(Gone);
            }
            self.backlog.push_back(message);
        }
    }

    fn notify(&mut self, text: &str) -> Result<(), Gone> {
        self.send(json!({"method": "notify", "params": {"text": text}}))
    }

    fn log(&mut self, message: &str) -> Result<(), Gone> {
        self.send(json!({"method": "log", "params": {"message": message}}))
    }

    fn run(&mut self) -> Result<(), Gone> {
        loop {
            let message = self.next()?;
            let params = &message["params"];
            match message.get("method").and_then(Value::as_str) {
                Some("shutdown") => return Ok(()),
                Some("command") if params["command"] == "export" => {
                    if let Some(text) = self.export()? {
                        self.notify(&text)?;
                    }
                }
                _ => {}
            }
        }
    }

    /// Returns the message to show, or None if the user cancelled.
    fn export(&mut self) -> Result<Option<String>, Gone> {
        let model = match self.request("doc.print_model", json!({}))? {
            Ok(model) => model,
            Err(e) => return Ok(Some(format!("Couldn't export: {e}"))),
        };
        let model: pdf::Model = match serde_json::from_value(model) {
            Ok(model) => model,
            Err(e) => return Ok(Some(format!("Couldn't export: Folio sent a print model this adapter can't read ({e})"))),
        };
        let title = if model.title.trim().is_empty() { "Untitled" } else { model.title.as_str() };
        let params = json!({"title": "Export as PDF", "file_name": format!("{title}.pdf"), "extension": "pdf"});
        let path = match self.request("ui.save_dialog", params)? {
            Ok(Value::String(path)) => PathBuf::from(path),
            Ok(Value::Null) => return Ok(None),
            Ok(other) => return Ok(Some(format!("Couldn't export: unexpected save dialog reply {other}"))),
            Err(e) => return Ok(Some(format!("Couldn't export: {e}"))),
        };
        Ok(Some(match write(&model, &path) {
            Ok(()) => format!("Exported {}", path.file_name().unwrap_or(path.as_os_str()).to_string_lossy()),
            Err(e) => format!("Couldn't export: {e}"),
        }))
    }
}

fn write(model: &pdf::Model, path: &Path) -> Result<(), String> {
    let bytes = pdf::render(model)?;
    std::fs::write(path, bytes).map_err(|e| format!("couldn't save {}: {e}", path.display()))
}

fn main() {
    let stdin = io::stdin();
    let mut conn = Conn { input: stdin.lock(), output: io::stdout().lock(), next_id: 0, backlog: VecDeque::new() };
    let _ = conn.run();
}
