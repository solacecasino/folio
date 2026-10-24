# folio_plugin

Folio's embedded plugin runtime. Plugins are WebAssembly components
(WASI Preview 2) run by [Wasmtime](https://wasmtime.dev) inside Folio,
each in a sandbox that grants nothing it didn't ask for.

| | |
|---|---|
| Interface | [`wit/folio-plugin.wit`](wit/folio-plugin.wit): read the selection and document metadata, insert text, log, notify; handle `startup`, commands, `changed`, `saved`, `selection-changed` |
| Sandbox | No filesystem, network, environment or document access unless `plugin.toml` asks. Document access is checked per call; folders become WASI preopens; network grants become a socket address allow-list |
| Limits | Time per event (epoch interruption stops even infinite loops), memory, and edits/messages per event. Crashed plugins restart; ones that keep failing are turned off |
| Editor API | `PluginRuntime` runs plugins on a background thread: `set_document`, `dispatch`, `run_command` return immediately, and `poll` collects edits and messages each frame |

## A plugin

```text
~/.folio/plugins/word-goals/
├── plugin.toml
└── word_goals.wasm      cargo build --target wasm32-wasip2 --release
```

```toml
name = "Word Goals"
version = "0.1.0"
wasm = "word_goals.wasm"
events = ["changed"]

[[commands]]
id = "set-goal"
label = "Set Word Goal"

[capabilities]                       # everything is denied unless listed
document = ["read", "edit"]
filesystem = [{ path = "~/Notes", access = "read" }]   # appears as /Notes
network = ["api.example.com:443"]    # or "any"
```

In Rust, a plugin is a `cdylib` using
[`wit-bindgen`](https://crates.io/crates/wit-bindgen) with this crate's
`wit/` folder; [`tests/guest`](tests/guest/src/lib.rs) is a complete one.
Any language that builds WASI Preview 2 components works.

## Using it from the editor

```rust
let mut plugins = PluginRuntime::start(RuntimeConfig {
    waker: Some(Arc::new(move || ctx.request_repaint())),
    ..Default::default()
})?;

plugins.set_document(snapshot);      // when the document or selection changes
plugins.dispatch(Event::Changed);
plugins.run_command("Word Goals", "set-goal");

for output in plugins.poll() {       // every frame; never waits
    match output {
        PluginOutput::Edit { revision, edit, .. } => { /* apply if still current */ }
        PluginOutput::Notify { message, .. } => { /* show it */ }
        PluginOutput::Loaded(info) => { /* add its commands to the menus */ }
        PluginOutput::Error(e) => { /* show it; the editor carries on */ }
        PluginOutput::Log { .. } => {}
    }
}
```

## Tests

```sh
rustup target add wasm32-wasip2
cargo test -p folio_plugin
```

The integration tests build `tests/guest` and check, from inside a real
plugin, that files, network, environment and document access are refused
unless granted, that runaway and crashing plugins are stopped and
restarted, and that the editor's calls never wait on a plugin.
