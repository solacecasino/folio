# Customizing Folio

Folio has no settings window. Everything is a text file in one folder:

```
~/Library/Application Support/Folio/
├── settings.toml     how Folio behaves and what's on screen
├── keys.toml         every keyboard shortcut
├── themes/           how documents look
└── adapters/         add-ons you chose to install
```

Open it with **Folio → Open Config Folder**. Folio reloads these files as soon
as you save them; **Folio → Reload Config** (`Cmd+Shift+R`) forces a reload.
If a file has a mistake, Folio keeps running with the defaults and shows the
problem in red at the bottom of the window.

Sharing that one folder shares your whole setup.

## settings.toml

```toml
theme = "Paper"            # starting theme (Cmd+T saves your choice here)
zoom = 1.0
show_status = true         # Cmd+.
show_panels = true         # Cmd+\
status = "%words% words"   # the status line, in the formatting language
title = "%file%$if(%dirty%, •,)"
```

### Writing

```toml
autosave = true            # save named documents 2 seconds after you stop typing
versions = 50              # snapshots kept per document; 0 turns it off
spellcheck = true          # the macOS spell checker, with your learned words
spell_language = "en"      # empty = your system language
smart_quotes = true        # "straight" becomes “curly” as you type
smart_dashes = true        # -- becomes —, ... becomes …
```

- **Autosave** only saves documents that already have a file. Unsaved work
  is kept in `recovery/` instead, and if Folio ever closes unexpectedly it
  offers that work back the next time it opens.
- **Versions** are written to `versions/` each time you save (and at most
  every five minutes while autosaving). **File → Browse Versions…** lists
  them; Restore can be undone with Cmd+Z. They're ordinary `.folio` files.
- Right-click a word with a red underline for suggestions, **Learn
  Spelling** (adds it to your Mac's dictionary) or **Ignore Spelling**.
- Cmd+Z right after a smart quote or dash gives back the plain character.

Every line is optional. Folio writes `theme`, `zoom`, `show_status`,
`show_panels` and `panels` back when you change them in the app, keeping your
comments.

## keys.toml

```toml
"cmd+shift+h" = "style h2"
"cmd+t" = "none"                                   # free a default shortcut
"cmd+alt+g" = 'adapter "Writing Tools" goal'
```

Modifiers: `cmd`, `shift`, `alt` (or `option`), `ctrl`. Keys: letters,
digits, `f1`–`f20`, punctuation such as `=` `-` `.` `/` `\`, and names such as
`enter`, `escape`, `up`, `space`. Your lines override the defaults, and the
menu bar shows your shortcuts. `Cmd+/` lists every current binding.

| Command | Does |
|---|---|
| `new` `open` `save` `save-as` `export-pdf` `versions` `quit` | Files |
| `undo` `redo` `select-all` | Editing |
| `find` `find-next` `find-previous` `replace` | Find & replace |
| `bold` `italic` `underline` | Inline formatting |
| `style <name>` | Apply a paragraph style from the theme |
| `next-theme` `zoom-in` `zoom-out` `zoom-reset` | View |
| `toggle-status` `toggle-panels` `help` | Show or hide things |
| `reload-config` `open-config-folder` | Config |
| `adapter "<Name>" <command-id>` | Run an adapter's command |

Themes add default `cmd+<digit>` shortcuts for styles with `key = <digit>`,
and adapters add the `keys` in their manifest. Your `keys.toml` overrides both.

## The formatting language

Used by `status`, `title`, text panels, and theme headers and footers. It's
modeled on foobar2000's title formatting.

| Syntax | Meaning |
|---|---|
| `%words%` | A field's value |
| `'text'` | Literal text, even with `%`, `$`, `[` in it (`''` is a quote) |
| `[ ... ]` | Shown only if a field inside has a value |
| `$if(cond,then,else)` | `then` if `cond` has a value, else `else` |
| `$if2(a,b)` | `a` if it has a value, else `b` |
| `$upper(x)` `$lower(x)` | Change case |
| `$plural(n,word)` | `word` or `words`; `$plural(n,child,children)` for irregular words |
| `$num(n,width)` | Zero-padded number |
| `$pad(x,width)` | Padded with spaces |

### Fields

| Field | Value |
|---|---|
| `%words%` `%chars%` `%paragraphs%` | Counts |
| `%page%` `%pages%` | Caret's page and page count (in headers and footers: that page) |
| `%reading_time%` | e.g. `4 min` |
| `%selection_words%` | Words selected (empty when nothing is) |
| `%file%` `%path%` | Document name, full path (empty if unsaved) |
| `%dirty%` | `1` with unsaved changes, else empty |
| `%theme%` `%style%` `%zoom%` | Current theme, paragraph style, zoom percent |
| `%date%` `%time%` | e.g. `2026-10-23`, `14:05` |
| `%adapter_status%` | Status text from all running adapters |
| `%adapter:writing tools%` | Status text from one adapter (lowercase name) |

Examples:

```toml
status = "%words% $plural(%words%,word)[ · %selection_words% selected]   p. %page%/%pages%"
title = "$upper(%file%)$if(%dirty%, — edited,)"
```

## Panels

Panels sit beside or below the page. There are none until you add some:

```toml
[[panels]]
kind = "outline"        # clickable headings
side = "left"           # left, right or bottom
size = 220

[[panels]]
kind = "text"           # your own lines in the formatting language
side = "right"
title = "Stats"
lines = ["%words% words", "%reading_time% to read", "page %page% of %pages%"]

[[panels]]
kind = "versions"       # this document's saved versions; click Restore

[[panels]]
kind = "adapter"        # filled by an adapter
adapter = "Writing Tools"
id = "goal"
```

Right-click a panel's title to move it to another side or close it, and drag
its edge to resize. Folio writes those changes back into settings.toml.
`Cmd+\` hides or shows all panels.

The outline lists `title`, `h1`, `h2` and `h3` paragraphs. A theme can change
that per style with `outline = <level>` (`0` leaves a style out).

## Themes

See [themes/paper.toml](../themes/paper.toml) for every option. Themes can
also set:

```toml
[page]
header = "%file%"
footer = "%page% / %pages%"
print_background = false   # true prints the page color in PDFs

[colors]
panel = "#2a2a2a"
panel_text = "#cccccc"
find = "#ffcc0055"         # highlight behind find matches
```
