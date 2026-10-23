# Folio components

Folio ships bare. Everything extra is a **component**: an optional add-on you
install on purpose, like foobar2000's components. A component is an ordinary
program in any language. Folio starts it only when it's needed and talks to it
with JSON messages, one per line, over stdin and stdout.

`components/writing-tools/` is a complete example in about 100 lines of Python.

## Installing

Put the component's folder in:

```
~/Library/Application Support/Folio/components/
```

Restart Folio, and a **Components** menu appears with the component's commands.
(With no components installed, there's no Components menu at all.) Folio also
reads `./components` next to where it runs, which is handy while developing.

The first time a component runs, Folio shows its name, author, license and
permissions and asks you to allow it. Your answer is remembered in
`~/Library/Application Support/Folio/approvals.json` until the component asks
for more permissions.

## `component.toml`

```toml
name = "Writing Tools"          # shown in the menu and approval dialog
version = "0.1.0"
author = "you"
license = "MIT"
description = "One sentence about what it does."

run = ["python3", "main.py"]    # program + arguments, run from this folder
permissions = ["document.read", "document.edit"]
start = "command"               # "command" (default) or "launch"
events = ["changed"]            # events to receive while running

[[commands]]
id = "goal"                     # sent back to you when chosen
label = "Cycle Word Goal"       # menu text
keys = "cmd+alt+g"              # optional default shortcut (users can remap it in keys.toml)

[[panels]]
id = "goal"                     # panels you can fill with panel.set
title = "Word Goal"
```

- `start = "command"` starts the component the first time you pick one of its
  commands. `start = "launch"` starts it when Folio opens; use it only if the
  component needs events from the start.
- When Folio quits it sends `shutdown`, then stops the program.
- Users place your panels in their `settings.toml`
  (`kind = "component"`, `component = "<name>"`, `id = "<panel id>"`). A
  component whose panel is on screen is started automatically, after approval.
- Users can bind your commands to their own keys in `keys.toml`:
  `"cmd+k" = 'component "Writing Tools" goal'`.

### Permissions

| Permission | Allows |
|---|---|
| `document.read` | `doc.get`, `doc.text` |
| `document.edit` | `doc.insert`, `doc.replace` |
| `network` | Nothing through Folio; it tells the user the component goes online |

Permissions control what a component can do **through Folio**. A component is a
normal program running as you, so `network` is an honest declaration, not a
sandbox. Only install components you trust; being open source is what lets
people check.

## Messages

Every message is one JSON object on one line.

- A **notification** has `method` and `params` and gets no reply.
- A **request** also has an `id`. The reply has the same `id` and either
  `result` or `error` (a string).

Only components send requests. Folio sends notifications.

### Folio → component

| Method | Params | When |
|---|---|---|
| `initialize` | `folio` (version), `protocol` (currently `2`), `permissions` | Right after starting |
| `command` | `command`: the command `id` | You picked one of its commands |
| `event` | `kind` plus the fields below | Something happened, if listed in `events` |
| `shutdown` | | Folio is quitting; exit soon |

Events:

| `kind` | Fields | Notes |
|---|---|---|
| `changed` | `words`, `paragraphs` | Right after starting, then at most about 3 times a second while typing |
| `saved` | `path` | |
| `opened` | `path` | |

### Component → Folio

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

Edits from components are single undo steps, so `Cmd+Z` undoes them.

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

## Versions

`initialize` says which protocol version Folio speaks. Version 2 added panels
(`panel.set`, `[[panels]]`). New versions only add things, so a component
written for version 1 keeps working.

## Example exchange

```
→ {"method":"initialize","params":{"folio":"0.1.0","protocol":1,"permissions":["document.read"]}}
→ {"method":"command","params":{"command":"stats"}}
← {"id":1,"method":"doc.get","params":{}}
→ {"id":1,"result":{"folio":1,"paragraphs":[...],"path":"/Users/me/essay.folio","theme":"Paper"}}
← {"method":"notify","params":{"text":"12 paragraphs, 3 headings, 840 words"}}
```

(`→` is Folio to the component, `←` is the component to Folio.)
