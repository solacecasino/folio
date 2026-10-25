# Extending Folio: plugins and adapters

Folio ships bare and has two ways to add to it. They're for different jobs.

| | **Plugins** (WebAssembly) | **Adapters** (separate programs) |
|---|---|---|
| For | Most extensions: text tools, UI tools, format parsers | Integrations with external programs and command-line tools, background work |
| Runs | Inside Folio, in a sandbox | As its own process, talking JSON over stdin/stdout |
| Can reach | Only what its manifest declares: the document, chosen folders, chosen network addresses | Anything the person's account can (permissions describe it, but don't enforce it) |
| Written in | Any language that builds a WASI Preview 2 component (Rust, C, Go, JavaScript, Python via componentize-py, ...) | Any language |
| Safety | Enforced: time, memory and output limits; crashes are contained | Trust: only install adapters you trust |
| Lives in | `plugins/` in the config folder | `adapters/` in the config folder |
| Spec | [crates/folio_plugin](../crates/folio_plugin/README.md), [WIT interface](../crates/folio_plugin/wit/folio-plugin.wit) | [docs/adapters.md](adapters.md) |

**Plugins are the main way to extend Folio.** A plugin can't read your files,
go online or even see your document unless it says so up front, which is
what makes installing other people's plugins safe. Reach for an adapter
when you need something a sandbox can't give: running `pandoc`, talking to a
long-lived peer-to-peer connection (Folio Link), or other system integration.

## Plugins

> **Plugins are a preview in Folio 1.0.** They work and are sandboxed, but
> the interface may still grow and change before it's declared stable.

**Install** a plugin by dropping its `.zip` (or folder) on Folio's window.
Folio shows what it can do first. **Make one** from
[folio-plugin-template](https://github.com/solacecasino/folio-plugin-template):
a working Rust plugin and a workflow that releases it on each version tag.

```text
plugins/word-goals/
├── plugin.toml
└── word_goals.wasm
```

The config folder is `~/Library/Application Support/Folio` on macOS,
`%APPDATA%\Folio` on Windows and `~/.config/folio` on Linux (Folio → Open
Config Folder).

```toml
name = "Word Goals"
version = "0.1.0"
wasm = "word_goals.wasm"
events = ["changed"]            # also "saved", "selection-changed"

[[commands]]                    # shown in the Plugins menu
id = "set-goal"
label = "Set Word Goal"

[capabilities]                  # everything is denied unless listed
document = ["read", "edit"]
filesystem = [{ path = "~/Notes", access = "read" }]   # the plugin sees /Notes
network = ["api.example.com:443"]                      # or "any"
```

Plugin commands appear in a **Plugins** menu and can be bound in
`keys.toml`: `"cmd+alt+g" = 'plugin "Word Goals" set-goal'`.

Plugins can read the selection and document facts, insert text or replace
a range, show text in the status line (`%plugin_status%`), fill panels they
declare with `[[panels]]` (placed with `kind = "plugin"` in settings.toml),
show messages, and log. **View → Log** shows their log lines and errors.
Folio → Reload Config reloads plugins too.

What a plugin can call, and the events it gets, are in the
[WIT interface](../crates/folio_plugin/wit/folio-plugin.wit). Edits a plugin
makes are applied by the editor after the event, as normal undo steps; an
edit made against a document you've since changed is skipped rather than
put in the wrong place.

Every plugin runs under limits it can't change: about 2 seconds per event,
64 MB of memory, and 1,000 edits or messages per event. A plugin that
crashes is restarted; one that fails three times in a row is turned off
until Folio restarts.

## folio-lite

The plugin runtime ([Wasmtime](https://wasmtime.dev)) is most of Folio's
size, so releases come in two editions:

| | Download (macOS) | |
|---|---|---|
| **Folio** | about 9 MB | Plugins and adapters |
| **Folio Lite** | about 5 MB | Adapters only; plugins are ignored |

Build either from source:

```sh
cargo build --release                         # Folio
cargo build --release --no-default-features   # Folio Lite
```

The `wasi-plugins` cargo feature (on by default) is the only difference.
Everything else, including `keys.toml` bindings to plugin commands, works
the same in both; in Folio Lite, running a plugin command says that this
edition doesn't include plugins.
