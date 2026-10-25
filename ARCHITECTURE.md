# Folio architecture

Folio is a page-based document editor in Rust, built on `eframe`/`egui` and
drawn with OpenGL through `glow`. It runs on macOS (Apple
Silicon), Windows and Linux (x86-64) from one codebase. This document explains
how it is put together and why. Paths are relative to the repository root.

---

## 1. Principles

### Start bare

A fresh install opens to a page and a caret. There is no ribbon, no
sidebar, no account and no start screen. Every feature that is not needed to
write, save and print text is either off by default (panels, status fields)
or not in the binary at all (PDF export, co-editing, lists and links
authoring), and arrives only when someone installs an extension.

This is a scope rule, not a styling rule. It keeps the core small enough to
reason about: the editor, its layout engine, its file format and the app shell are
about 4,300 lines of Rust, and everything else hangs off two narrow extension interfaces
(sections 3 and 4).

Folio draws its own UI with an immediate-mode toolkit rather than embedding a
browser. The consequences, in practice:

- **One process, one window per document.** No renderer process, no
  JavaScript heap, no IPC between UI layers. Downloads are 5–13 MB
  depending on platform and edition.
- **Idle means idle.** The app repaints when input arrives, when a timer it
  set expires (caret blink, autosave, config polling), or when a background
  thread wakes it. Nothing renders while nothing changes.
- **Native where it matters.** The macOS menu bar is the real NSMenu
  (`muda`), spell checking uses each OS's own checker, dialogs are native
  (`rfd`). Elsewhere Folio draws its own menus in the theme's colors.

### Configuration is files

There is no preferences window. Behaviour lives in `settings.toml`, shortcuts
in `keys.toml`, appearance in theme files, all in one config folder:

| OS | Folder |
|---|---|
| macOS | `~/Library/Application Support/Folio` |
| Windows | `%APPDATA%\Folio` |
| Linux | `$XDG_CONFIG_HOME/folio` |

`FOLIO_CONFIG_DIR` overrides it, which makes a portable setup a folder on a
USB stick.

Why files instead of a settings UI:

- **The state is the file.** There is no second representation to drift out
  of sync with what's on disk. What you see in the file is what Folio uses.
- **They diff, merge and travel.** A setup can be kept in Git, shared as a
  folder, or reviewed line by line. Copying a colleague's `keys.toml` is
  copying a file.
- **Live reload with a safe failure mode.** `settings::Watch` checks file
  modification times about once a second (no filesystem-notification
  dependency, no extra thread). A file that fails to parse doesn't half-apply:
  Folio keeps the last good values and shows the parse error, with its line,
  in the status area until the file is fixed.
- **Writes don't destroy hand edits.** When Folio itself changes a setting
  (the last theme, panel sizes), it edits the TOML with `toml_edit`, which
  keeps comments, ordering and formatting.

Things that are memory rather than preference — window placement, recent
documents — go in a separate `state.json` so the files people edit stay
theirs.

Folio does write outside its folder in two places, both to make the OS hand
documents and links to it: per-user file and URL associations in the Windows
registry (`HKCU\Software\Classes`, no administrator rights), and, when the
user runs `install.sh`, a desktop entry and MIME type on Linux.

### The user owns the data

- **No network.** The core never opens a socket. Updates, the add-on
  catalog and documentation are links that open in the user's browser.
  Extensions that go online must say so up front (sections 3, 4).
- **No telemetry, no accounts, no cloud.**
- **Open format.** A `.folio` file is pretty-printed JSON:

  ```json
  {
    "folio": 1,
    "theme": "Paper",
    "paragraphs": [
      { "style": "h1", "runs": [{ "text": "Notes" }] },
      { "style": "body", "list": { "kind": "bullet", "level": 0 },
        "runs": [{ "text": "See " }, { "text": "the spec", "link": "https://example.com" }] }
    ]
  }
  ```

  It is readable without Folio, diffs cleanly, and is easy for other tools to
  generate. New fields are additive and optional, so older builds open newer
  files and ignore what they don't know.
- **A safety net that is also plain files.** Unsaved work is mirrored to
  `recovery/<pid>.json` two seconds after typing stops; a recovery file whose
  process is no longer alive means a crash, and Folio offers it back on next
  launch. Each save also writes a snapshot to `versions/<doc id>/<time>.folio`,
  capped per document. All of it is ordinary `.folio` you can open directly.

---

## 2. Components

```
src/                     the app (one binary)
  main.rs                app state, frame loop, commands, file I/O
  editor.rs              layout, pagination, input, painting, print model
  doc.rs                 document model and .folio format
  theme.rs               themes, fonts, list and link styling
  settings.rs keys.rs    settings.toml, keys.toml, live reload
  format.rs              the formatting language (%words%, $if, ...)
  menu.rs                native menu bar (macOS) or drawn menus
  panels.rs find.rs      side panels, find and replace
  spell.rs               spell checking per OS
  safety.rs state.rs     versions, crash recovery, window state, recents
  adapters.rs            out-of-process adapter host
  plugins.rs             bridge to the WebAssembly runtime (optional)
  links.rs platform.rs   OS integration: open events, paths, trash, time
crates/folio_plugin/     WebAssembly plugin runtime (Wasmtime)
themes/                  bundled themes (TOML)
scripts/ packaging/      build and packaging
```

### The app

`main.rs` holds one `Folio` struct and runs once per frame:

1. drain commands from the menu bar and the keymap,
2. service adapters and plugins (non-blocking polls),
3. run the safety net and config watcher,
4. lay out and paint the page, panels and overlays.

All mutable state is owned by that struct and touched only on the UI thread.
Background threads exist only at the edges — reading adapter stdout, running
plugins, ticking the plugin epoch — and they talk to the UI thread through
channels plus a repaint request. There are no locks around document state
because nothing else can reach it.

Platform differences are kept in a handful of places instead of spread
through the code: `platform.rs` (config path, local time, process liveness,
opening URLs, trash), `spell.rs` (three backends behind one API),
`menu.rs` (native vs drawn), `links.rs` (Apple Events vs command-line
arguments). Key bindings are written once (`cmd+s`) and mean Cmd on macOS and
Ctrl elsewhere, so a `keys.toml` is portable.

### Themes

A theme is a TOML file that defines the page (size and margins in inches),
colors, font families, list styling and paragraph styles (size, line height,
spacing, alignment, indent, keyboard shortcut, the style after Enter). The
engine has no built-in notion of "heading" or "quote": they are names in the
theme, and the layout engine reads their numbers directly.

Fonts are files. A face can name a list of candidates —
`["Georgia.ttf", "georgia.ttf", "DejaVuSerif.ttf"]` — and the first one
present on the machine wins; bare file names are looked up in the system
font folders. That is how one theme renders sensibly on all three platforms
without a font-matching service. Every face of every theme is registered with
the text renderer at startup, with the built-in font as a fallback for
missing glyphs, so switching themes never waits on font loading.

### Packaging

`scripts/package.py` builds the release for the current OS and packs it; CI
runs it on macOS, Windows and Linux on every version tag and attaches the
results to the GitHub release.

| | Ships as | Notes |
|---|---|---|
| macOS | `Folio.app` in a zip | ad-hoc signed (not notarized) |
| Windows | `folio.exe` + `themes/` in a zip | runs from any folder; icon embedded at build |
| Linux | binary + `themes/` + `install.sh` in a tarball | needs the usual X11/Wayland and GL libraries |

Each comes in two editions from the same source: **Folio** and **Folio
Lite**, which is built with `--no-default-features` and leaves out the
WebAssembly runtime (Wasmtime is the bulk of the binary). The `wasi-plugins`
feature is the only difference; `plugins.rs` has the same API in both builds,
so no other code checks the feature. Release builds use LTO with one codegen
unit and are stripped.

Nothing is installed system-wide. `install.sh` writes to `~/.local`, and
uninstalling is deleting files.

---

## 3. Plugins: sandboxed WebAssembly

`crates/folio_plugin` runs third-party code inside Folio's process. Plugins
are WebAssembly components (WASI Preview 2) executed by Wasmtime; the
contract is `wit/folio-plugin.wit`.

### Why WebAssembly

The requirement was in-process extensions that people can install from
strangers without reading their source. Native dynamic libraries share the
host's address space and privileges, so a bug or a malicious plugin owns the
editor. Scripting languages embedded in-process need their own sandbox and
usually leak capabilities through their standard library. WebAssembly gives:

- **Memory isolation by construction.** A plugin addresses only its own
  linear memory; it cannot read Folio's.
- **Capabilities, not ambient authority.** A component can only call what it
  imports, and Folio decides what each import does. There is no syscall
  escape hatch.
- **Any source language** that targets `wasm32-wasip2` (Rust, C, Go,
  JavaScript, Python through componentize-py).
- **One binary for every OS.** A plugin is a single `.wasm`, so it ships as
  one zip instead of one per platform.

### The boundary

What a plugin can do is exactly the WIT interface:

- **Read** the selection and document metadata, from a snapshot the editor
  published with the event (`document.read`).
- **Change** the document by inserting text at the caret or replacing a
  range (`document.edit`).
- **Show** text in the status line, fill panels it declared, notify, log.
- **Receive** `startup`, its own commands, `changed`, `saved`,
  `selection-changed`.

Plugins can't draw on the page or open windows; that is deliberate, since
it keeps them from spoofing Folio's own UI.

Capabilities are declared in `plugin.toml` and default to nothing:

```toml
[capabilities]
document = ["read", "edit"]
filesystem = [{ path = "~/Notes", access = "read" }]   # visible as /Notes
network = ["api.example.com:443"]                      # or "any"
```

Folio shows this list before installing. Enforcement happens in two layers:

1. **Document access** is checked on every host call
   (`host.rs`). A plugin without `document.read` gets `missing-capability`.
2. **System access** is a WASI context built from the manifest
   (`sandbox.rs`): no preopened directories unless granted (so no filesystem
   at all by default), no environment variables, no arguments. Network
   access is off unless declared; a host list is resolved at start and
   enforced per socket address, including refusing listeners. TCP, UDP and
   DNS switches are set explicitly rather than trusting library defaults,
   because those defaults have differed from their documentation.

Unknown keys in `plugin.toml` are errors, so a typo cannot silently grant or
drop a capability.

### Resource limits

A sandbox that can't stop runaway code isn't one. Every plugin runs under
limits it cannot change:

| Limit | Default | Mechanism |
|---|---|---|
| Time per event | 2 s | Wasmtime epoch interruption: a ticker thread advances the epoch every 10 ms; at each tick the plugin yields, and past its deadline it traps |
| Memory | 64 MB | `StoreLimits`; growth past it traps |
| Output per event | 1,000 messages, 4 MB of text | counted in the host functions |

A trap discards the instance (its state can't be trusted) and the next event
gets a fresh one. Three consecutive failures turn the plugin off until a
reload. A plugin's own returned error is not a crash and keeps its state.

### Never blocking the editor

Plugins run on one background thread with a single-threaded Tokio runtime,
one task per plugin, each with its own queue. The editor's side
(`PluginRuntime`) never waits:

- `set_document` publishes an `Arc` snapshot; each dispatched event carries
  the snapshot current at dispatch, so a plugin sees a consistent document
  while the user keeps typing.
- `dispatch` and `run_command` enqueue and return.
- `poll` drains finished outputs once per frame.

If a plugin falls behind, queued `changed`/`selection-changed` events
collapse to the newest; commands and saves are never dropped. Edits come back
tagged with the document revision they were made against; the editor applies
them only if the document hasn't changed since (a plugin's several edits from
one event apply in order), so a slow plugin can never write into the wrong
place.

Plugins are a preview in 1.0: the interface may still grow.

---

## 4. Adapters: separate programs over stdio

Adapters are the other extension path, for what a sandbox should not or
cannot do: running external tools, holding long-lived network connections
(Folio Link, peer-to-peer co-editing), heavy work like PDF generation.

### The protocol

An adapter is a folder with `adapter.toml` and a program. Folio starts the
program on first use, in its folder, and exchanges JSON, one object per line,
over stdin and stdout (`docs/adapters.md`). A message with an `id` is a
request and gets a reply with the same `id`; without one it's a notification.
Folio only sends notifications (`initialize`, `command`, `event`,
`shutdown`); adapters send requests (`doc.get`, `doc.insert`,
`doc.set_list`, `ui.prompt`, ...).

That is the whole contract. No shared library, no ABI, no SDK, no
code generation. Anything that can read a line and print a line is an
adapter, in any language. With the one-file Python helper in
`folio-adapters/sdk/python/folio.py`, a working adapter is about ten lines:

```python
import folio

@folio.command("count")
def count():
    folio.notify(f"{len(folio.text().split())} words")

folio.run()
```

The protocol is versioned (currently 5) and only ever adds; an adapter
written for version 1 still works.

### Isolation

An adapter is a separate OS process, so:

- **A crash is contained.** If the adapter dies, its stdout closes; the
  reader thread reports it, the host marks the adapter stopped, and the next
  command starts it again. The editor's memory is never shared with it.
- **A hang is contained.** Adapter output is read on its own thread and
  handed to the UI as messages, so a slow or stuck adapter can't freeze
  typing. Requests that need the user (`ui.prompt`, the PDF print model) are
  answered asynchronously, from a later frame.
- **Nothing runs until it's used.** Adapters start on their first command
  (or at launch only if they declare they need events), and Folio Link opens
  no socket until the user chooses Share or Join.

What an adapter is not is sandboxed. It runs with the user's permissions.
Its declared permissions (`document.read`, `document.edit`, `network`) gate
what it can ask *Folio* to do, are shown before it first runs, and are
remembered per adapter until they change, but they cannot stop the program
from touching the filesystem on its own. That trade is the reason both
extension types exist: plugins for untrusted code, adapters for trusted code
that needs the whole machine.

### Distribution

Compiled adapters ship one zip per platform; script adapters one zip for all.
A reusable GitHub workflow builds and names them, and the
[catalog](https://github.com/solacecasino/folio-adapters/blob/main/CATALOG.md)
is generated from each listed repository's latest release, reading the
manifest inside the actual zip, so the permissions shown in the catalog are
the ones Folio will ask about. Folio refuses a zip built for another OS and
names the right one.

---

## 5. Layout and pagination

Folio lays out real pages — size, margins, page breaks — on its own, with no
HTML engine. Text shaping and line breaking come from `epaint` (egui's
renderer); everything above the line is Folio's (`editor.rs`).

### Paragraphs to rows

Each paragraph becomes one `LayoutJob`: its runs, each with a text format
derived from the theme style and the run's formatting (bold, italic,
underline, link). The wrap width is the page width minus both margins, minus
the list indent for list items. `epaint` shapes and wraps it into a *galley*:
a list of rows, each with its glyph positions and a `(min_y, max_y)` extent.
Galleys are cached by the hash of their job, so on a typical frame only the
paragraph being edited is reshaped.

List items are indented `(level + 1) × indent` and get a marker laid out as
its own small galley, right-aligned just left of the text. Numbers are
computed for the whole document in one pass (`Document::list_numbers`):
counters per level, deeper counters reset when returning to a shallower
level, all reset by a non-list paragraph.

### Rows to pages

`paginate` takes a paragraph's row extents, the page it starts on and its y,
and splits the rows into **pieces**:

```
offset = y - rows[0].min           (galley-local 0 in page coordinates)
for each row i:
    if i is not the first row of this piece
       and offset + rows[i].max > page_bottom:
        close the piece at i on this page
        page += 1
        offset = margin_top - rows[i].min
```

Each piece records its row range, its page and where the galley's origin
sits on that page. Three rules make it robust:

- **A row is never split.** Breaks happen between rows.
- **Progress is guaranteed.** The first row of a piece is always placed even
  if it is taller than the page, so an oversized row can't loop forever.
- **Whole paragraphs move first.** Before paginating, if not even the first
  row fits under the previous paragraph, the paragraph starts on a new page,
  and `space_before` is dropped at the top of a page.

A paragraph is shaped **once**, not once per page. Each piece draws the same
galley translated to its origin and clipped to its own rows. That keeps
selection, caret placement and hit-testing in one coordinate system per
paragraph: a position maps to a galley cursor, the cursor to a row, the row
to the piece that holds it.

### Cost

Layout is linear in the number of rows: one pass for numbering, one shaping
call per paragraph (mostly cache hits), one walk over the row extents. The
document is re-laid-out each frame it is shown, which is simple and, with the
galley cache, cheap for documents of normal length; very long documents are
where an incremental layout would pay off.

Undo uses whole-document snapshots, capped at 500 steps, with typing grouped
into one step. Memory per step is the document's size, which is the cost of
undo being trivially correct.

### Printing is the same layout

The PDF path doesn't have its own typesetter. `Editor::print_model` runs the
same layout at 100% zoom and converts it to PDF points (72/96): words at
their baselines with their font files, underline rectangles, list markers,
and one rectangle per line of each link. The PDF Export adapter turns that
into a PDF with `krilla`, embedding the theme's fonts and adding link
annotations. What prints is what was on screen, line for line.

One consequence: a character the theme's font lacks is drawn on screen with
the fallback font but has no glyph in the PDF. Folio doesn't yet carry the
fallback chain into the print model.
