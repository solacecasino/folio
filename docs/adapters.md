# Folio adapters

> Most extensions should be [plugins](plugins.md): sandboxed WebAssembly that
> runs inside Folio and can only reach what it declares. Adapters are for
> what a sandbox can't do, such as running external command-line tools or
> keeping a background connection open.

Folio ships bare. An **adapter** is an optional add-on you install on
purpose, like foobar2000's components. An adapter is an ordinary
program in any language. Folio starts it only when it's needed and talks to it
with JSON messages, one per line, over stdin and stdout.

Folio's own adapters live in their own repo,
[folio-adapters](https://github.com/solacecasino/folio-adapters):

| Adapter | What it adds | Written in |
|---|---|---|
| `pdf-export` | File → Export as PDF… (Cmd+E), print-perfect | Rust |
| `folio-link` | Live co-editing with people you invite ([how it works](https://github.com/solacecasino/folio-adapters/blob/main/docs/folio-link.md)) | Rust |
| `writing-tools` | A word-count goal, today's date, document stats | Python, about 100 lines |

[writing-tools](https://github.com/solacecasino/folio-adapters/tree/main/writing-tools)
is the simplest example to start from.

Adapters used to be called *components*. Old `components/` folders,
`component.toml` files, `kind = "component"` panels and
`component "Name" id` key bindings all still work.

## Installing

Folio ships with no adapters. Open **Folio → Adapters…** (`Cmd+Shift+A`),
click **Add Adapter…**, and pick an adapter's `.zip` (or, below the list, a
folder). Dropping a zip or folder on any Folio window does the same. Folio shows what it is and what it may do before adding it, and it
works right away, with no restart. The window lists each adapter with its
permissions, a button for each command, and **Remove**.

Installed adapters live in the `adapters` folder of Folio's config folder
(**Show Folder** in the window). To package your own, zip its folder: a zip
holding one folder that contains `adapter.toml`. **Remove** moves an adapter
to the Trash (the Recycle Bin on Windows).

Commands appear in the Adapters window. A command can also ask for a place in
one of Folio's menus (`menu = "file"`, see below), and users can bind any
command to keys.

The first time an adapter runs, Folio shows its name, author, license and
permissions and asks you to allow it. Your answer is remembered in
`approvals.json` in the config folder until the adapter asks for more
permissions.

## Publishing

The quickest way to make and share an adapter:

1. **Start from [folio-adapter-template](https://github.com/solacecasino/folio-adapter-template)**
   ("Use this template" on GitHub). It's a working Python adapter, with
   [`folio.py`](https://github.com/solacecasino/folio-adapters/blob/main/sdk/python/folio.py),
   a one-file helper that handles the messages below, so a command is a few lines.
2. **Try it** with **Add an adapter from a folder instead…** in the Adapters
   window, or by dropping the folder on Folio. `.git`, `target/` and `dist/`
   aren't copied.
3. **Push a version tag** (`git tag v0.1.0 && git push origin v0.1.0`). The
   template's workflow publishes a release with correctly named zips: one
   for a script adapter, or one per platform (macOS, Windows, Linux) when
   there's a `Cargo.toml`. Any repo can use the same workflow:
   `uses: solacecasino/folio-adapters/.github/workflows/build-adapter.yml@main`.
4. **List it** in the [catalog](https://github.com/solacecasino/folio-adapters/blob/main/CATALOG.md)
   with the [List my adapter](https://github.com/solacecasino/folio-adapters/issues/new?template=list-my-adapter.yml)
   form, or a pull request adding `catalog/<name>.toml` with `repo = "you/name"`.
   The catalog reads everything else from your latest release, so new
   releases show up there by themselves.

Release zips are named `<adapter>-<version>-<platform>.zip` (`macos-arm64`,
`windows-x64`, `linux-x64`), or `<adapter>-<version>.zip` for adapters that
run everywhere. If someone adds a zip built for another kind of computer,
Folio tells them which one to download instead.

## `adapter.toml`

```toml
name = "Writing Tools"          # shown in the menu and approval dialog
version = "0.1.0"
author = "you"
license = "MIT"
description = "One sentence about what it does."
homepage = "https://github.com/you/writing-tools"   # shown as a link in the Adapters window

run = ["python3", "main.py"]    # program + arguments, run from this folder
permissions = ["document.read", "document.edit"]
start = "command"               # "command" (default) or "launch"
events = ["changed"]            # events to receive while running

[[commands]]
id = "goal"                     # sent back to you when chosen
label = "Cycle Word Goal"       # menu text
keys = "cmd+alt+g"              # optional default shortcut (users can remap it in keys.toml)
# menu = "file"                 # optional: show it in File, Edit, Format or View instead

[[panels]]
id = "goal"                     # panels you can fill with panel.set
title = "Word Goal"
```

- `run` is started in the adapter's folder. A program given as a path
  (`./pdf-export`) is found there, and on Windows `.exe` is added if needed,
  so one `adapter.toml` works on every platform. On Windows, `python3` also
  tries the `py` launcher and `python`.
- Folio sets `FOLIO_CONFIG_DIR` to its config folder, for adapters that keep
  their own files (Folio Link keeps its identity in `folio-link/` there), and
  `PYTHONIOENCODING=utf-8`.
- Compiled adapters are built once per platform. folio-adapters names each
  zip for its platform, such as `pdf-export-0.2.0-windows-x64.zip`.
- `start = "command"` starts the adapter the first time you pick one of its
  commands. `start = "launch"` starts it when Folio opens; use it only if the
  adapter needs events from the start.
- When Folio quits it sends `shutdown`, then stops the program.
- Users place your panels in their `settings.toml`
  (`kind = "adapter"`, `adapter = "<name>"`, `id = "<panel id>"`). An
  adapter whose panel is on screen is started automatically, after approval.
- Users can bind your commands to their own keys in `keys.toml`:
  `"cmd+k" = 'adapter "Writing Tools" goal'`.

### Permissions

| Permission | Allows |
|---|---|
| `document.read` | `doc.get`, `doc.text`, `doc.print_model`, `snapshot` events |
| `document.edit` | `doc.insert`, `doc.replace`, `doc.merge` |
| `network` | Nothing through Folio; it tells the user the adapter goes online |

Permissions control what an adapter can do **through Folio**. An adapter is a
normal program running as you, so `network` is an honest declaration, not a
sandbox. Only install adapters you trust; being open source is what lets
people check.

## Messages

Every message is one JSON object on one line, in UTF-8.

- A **notification** has `method` and `params` and gets no reply.
- A **request** also has an `id`. The reply has the same `id` and either
  `result` or `error` (a string).

Only adapters send requests. Folio sends notifications.

### Folio → adapter

| Method | Params | When |
|---|---|---|
| `initialize` | `folio` (version), `protocol` (currently `3`), `permissions` | Right after starting |
| `command` | `command`: the command `id` | You picked one of its commands |
| `event` | `kind` plus the fields below | Something happened, if listed in `events` |
| `undo`, `redo` | | During a shared session you started with `session.set` and `handles_undo: true`, instead of Folio's own undo |
| `shutdown` | | Folio is quitting; exit soon |

Events:

| `kind` | Fields | Notes |
|---|---|---|
| `changed` | `words`, `paragraphs` | Right after starting, then at most about 3 times a second while typing |
| `saved` | `path` | |
| `opened` | `path` | |
| `snapshot` | `revision`, `paragraphs`, `cursor`, `anchor` | The whole document on every change and caret move, right away. Needs `document.read`. For sync adapters. |

Positions (`cursor`, `anchor`) are `{ "para": 0, "ch": 12 }`: a paragraph
index and a character index within it.

### Adapter → Folio

| Method | Params | Result | Permission |
|---|---|---|---|
| `doc.get` | | The document (see below) plus `path` and `theme` | `document.read` |
| `doc.text` | | `{ "text": "..." }`, paragraphs separated by `\n` | `document.read` |
| `doc.insert` | `text` | `null`. Inserts at the caret, replacing any selection. `\n` starts a new paragraph | `document.edit` |
| `doc.replace` | `paragraphs` | `null`. Replaces the whole document | `document.edit` |
| `status.set` | `text` (string or `null`) | `null`. Shows text in the status line; `null` clears it | none |
| `notify` | `text` | `null`. Shows a short message for a few seconds | none |
| `panel.set` | `id`, `lines` | `null`. Fills one of your panels. Each line is a string, or `{ "text": ..., "command": ... }` for a clickable line that runs one of your commands | none |
| `log` | `message` | Printed to Folio's stderr, for debugging | none |
| `doc.print_model` | | The document laid out for printing (see below) | `document.read` |
| `doc.merge` | `base_revision`, `paragraphs`, `cursor`, `anchor` | `{ "applied": bool }`. Replaces the document only if `base_revision` is the current revision; otherwise wait for the next `snapshot` and try again. Not an undo step, and it clears Folio's undo history (undo belongs to the session). | `document.edit` |
| `session.set` | `active`, `handles_undo` | `null`. Starts or ends a shared session; with `handles_undo`, Folio sends you `undo`/`redo` | none |
| `presence.set` | `peers: [{ name, color, cursor, anchor }]` | `null`. Shows other people's carets and selections in their colors | none |
| `ui.confirm` | `title`, `message` | `true` or `false` from a native Yes/No dialog | none |
| `ui.save_dialog` | `title`, `file_name`, `extension` | The chosen path, or `null` if cancelled. Starts next to the document. | none |

Edits from adapters are single undo steps, so `Cmd+Z` undoes them.

### Document format

The same shape as a `.folio` file:

```json
{
  "paragraphs": [
    { "style": "h1", "runs": [{ "text": "Title" }] },
    { "style": "body", "runs": [
      { "text": "Some " },
      { "text": "bold", "bold": true },
      { "text": " text." }
    ]}
  ]
}
```

`style` is a paragraph style name from the active theme. Runs can have `bold`,
`italic` and `underline`.

### Print model

`doc.print_model` returns the document exactly as it lays out on the page, so
an adapter can print, export or preview it without knowing anything about
fonts or line breaking. Units are PDF points from each page's top left, `y` is
the text baseline, and every `font` is a file path you can read (Folio's
built-in font included).

```json
{
  "title": "essay", "width": 612.0, "height": 792.0, "background": null,
  "pages": [{
    "texts": [{ "x": 72.0, "y": 100.0, "text": "Hello", "font": "/System/Library/Fonts/Supplemental/Georgia.ttf",
                "size": 12.0, "color": "#222222ff", "slant": false }],
    "rects": [{ "x": 72.0, "y": 102.0, "w": 40.0, "h": 1.0, "color": "#222222ff" }]
  }]
}
```

`slant` means the theme has no italic file, so lean the regular face. `rects`
are underlines. `background` is the page color when the theme sets
`print_background = true`.

## Versions

`initialize` says which protocol version Folio speaks. New versions only add
things, so an adapter written for an older version keeps working.

- **2** added panels (`panel.set`, `[[panels]]`).
- **3** added printing (`doc.print_model`, `ui.save_dialog`), shared sessions
  (`snapshot`, `doc.merge`, `session.set`, `presence.set`, `undo`/`redo`),
  `ui.confirm`, and `menu =` for commands.

## Example exchange

```
→ {"method":"initialize","params":{"folio":"0.1.0","protocol":1,"permissions":["document.read"]}}
→ {"method":"command","params":{"command":"stats"}}
← {"id":1,"method":"doc.get","params":{}}
→ {"id":1,"result":{"folio":1,"paragraphs":[...],"path":"/Users/me/essay.folio","theme":"Paper"}}
← {"method":"notify","params":{"text":"12 paragraphs, 3 headings, 840 words"}}
```

(`→` is Folio to the adapter, `←` is the adapter to Folio.)
