//! Drives the built adapter with a fake Folio over stdin/stdout.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

const GEORGIA: &str = "/System/Library/Fonts/Supplemental/Georgia.ttf";

fn model() -> Value {
    let page = |word: &str, slant: bool| {
        json!({
            "texts": [{"x": 72.0, "y": 100.0, "text": word, "font": GEORGIA, "size": 12.0,
                       "color": "#222222ff", "slant": slant}],
            "rects": [{"x": 72.0, "y": 102.0, "w": 40.0, "h": 1.0, "color": "#222222ff"}]
        })
    };
    json!({"title": "essay", "width": 612.0, "height": 792.0, "background": "#fffaf0ff",
           "pages": [page("Hello", false), page("World", true)]})
}

#[test]
fn exports_through_a_fake_folio() {
    let dir = std::env::temp_dir().join(format!("pdf-export-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("essay.pdf");
    let _ = std::fs::remove_file(&out);

    let mut child = Command::new(env!("CARGO_BIN_EXE_pdf-export"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut send = |message: Value| writeln!(stdin, "{message}").unwrap();

    send(json!({"method": "initialize", "params": {"folio": "0.3.0", "protocol": 3, "permissions": ["document.read"]}}));
    send(json!({"method": "command", "params": {"command": "export"}}));

    let mut asked = Vec::new();
    let notice = loop {
        let mut line = String::new();
        assert!(stdout.read_line(&mut line).unwrap() > 0, "adapter exited early");
        let message: Value = serde_json::from_str(&line).unwrap();
        let method = message["method"].as_str().unwrap().to_owned();
        match method.as_str() {
            "doc.print_model" => send(json!({"id": message["id"], "result": model()})),
            "ui.save_dialog" => {
                assert_eq!(message["params"]["file_name"], "essay.pdf");
                assert_eq!(message["params"]["extension"], "pdf");
                send(json!({"id": message["id"], "result": out.to_str().unwrap()}));
            }
            "notify" => break message["params"]["text"].as_str().unwrap().to_owned(),
            other => panic!("unexpected {other}: {message}"),
        }
        asked.push(method);
    };
    assert_eq!(asked, ["doc.print_model", "ui.save_dialog"]);
    assert_eq!(notice, "Exported essay.pdf");

    send(json!({"method": "shutdown"}));
    drop(send);
    drop(stdin);
    assert!(child.wait().unwrap().success());

    let bytes = std::fs::read(&out).expect("the PDF was written");
    let text = String::from_utf8_lossy(&bytes);
    assert!(bytes.starts_with(b"%PDF-"));
    assert!(text.contains("/Type/Pages/Count 2"), "two pages");
    assert!(text.contains("/Title(essay)"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cancelling_the_save_dialog_writes_nothing() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pdf-export"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    writeln!(stdin, "{}", json!({"method": "command", "params": {"command": "export"}})).unwrap();
    for expected in ["doc.print_model", "ui.save_dialog"] {
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let message: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(message["method"], expected);
        let result = if expected == "doc.print_model" { model() } else { Value::Null };
        writeln!(stdin, "{}", json!({"id": message["id"], "result": result})).unwrap();
    }
    writeln!(stdin, "{}", json!({"method": "shutdown"})).unwrap();
    drop(stdin);
    let mut rest = String::new();
    std::io::Read::read_to_string(&mut stdout, &mut rest).unwrap();
    assert_eq!(rest, "", "no notify after a cancel");
    assert!(child.wait().unwrap().success());
}
