# Customizing Folio

Folio has no settings window. Everything is a text file in one folder:

```
Folio/
├── settings.toml     how Folio behaves and what's on screen
├── keys.toml         every keyboard shortcut
├── init.lua          your own commands, in Lua (optional)
├── themes/           how documents look
└── adapters/         add-ons you chose to install
```

| | The folder |
|---|---|
| macOS | `~/Library/Application Support/Folio` |
| Windows | `%APPDATA%\Folio` |
| Linux | `$XDG_CONFIG_HOME/folio`, usually `~/.config/folio` |

Set `FOLIO_CONFIG_DIR` to use another folder, such as a portable setup on a
USB stick. Open it with **Folio → Open Config Folder**. Folio also keeps
`state.json` there (window size and place, File → Open Recent), which you
don't need to edit. Folio reloads these files as soon
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
spellcheck = true          # your system's spell checker, with your learned words
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
  Spelling** or **Ignore Spelling**.
- Spell check uses the system's checker on macOS and Windows (add languages
  in the system settings), so learned words are shared with other apps. On
  Linux it uses Hunspell dictionaries: install your language's package (for
  example `hunspell-en-us`), or put `<lang>.aff` and `<lang>.dic` in the
  `dictionaries/` folder. Learned words go to `dictionaries/personal.txt`.
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

Modifiers: `cmd`, `shift`, `alt` (or `option`), `ctrl`. On Windows and Linux
`cmd` is the Ctrl key, so one `keys.toml` works on every computer. Keys: letters,
digits, `f1`–`f20`, punctuation such as `=` `-` `.` `/` `\`, and names such as
`enter`, `escape`, `up`, `space`. Your lines override the defaults, and the
menus show your shortcuts. `Cmd+/` lists every current binding.

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
| `log` | Help → Log: adapters' and plugins' output and errors |
| `guide` | Help → User Guide: opens the guide in your browser |
| `report-problem` | Help → Report a Problem…: copies debug info and opens the problem form |
| `adapters` `open-adapters-folder` | The Adapters window (Cmd+Shift+A), the folder they're kept in |
| `adapter "<Name>" <command-id>` | Run an adapter's command |
| `lua <name>` | Run a command your `init.lua` registered |

Themes add default `cmd+<digit>` shortcuts for styles with `key = <digit>`,
and adapters add the `keys` in their manifest. Your `keys.toml` overrides both.

## init.lua

Put an `init.lua` in the folder to add commands of your own, written in
[Lua 5.4](https://www.lua.org/manual/5.4/). Folio runs it once when it
starts, again whenever you save it, and on **Reload Config** (`Cmd+Shift+R`).
Without the file, Folio doesn't start Lua at all. Folio puts a working
`init.lua.example` in the folder: rename it to `init.lua` to start from it.
Registered commands appear in a **Scripts** menu, and `Cmd+/` lists the ones
that don't have a key yet.

```lua
-- Insert today's date where the caret is. The Scripts menu shows "Insert Date".
folio.register_command("date", function()
  folio.insert_text(os.date("%Y-%m-%d"))
end, "Insert Date")

-- Make the whole document upper case.
folio.register_command("shout", function()
  folio.set_text(folio.get_text():upper())
end)

-- Wrap the selection in quotes.
folio.register_command("quote", function()
  local _, _, text = folio.get_selection()
  folio.insert_text("“" .. text .. "”")
end)
```

Then bind them in `keys.toml`:

```toml
"cmd+shift+d" = "lua date"
"cmd+shift+u" = "lua shout"
"cmd+shift+q" = "lua quote"
```

| Function | Does |
|---|---|
| `folio.get_text()` | The document's text, paragraphs separated by `\n` |
| `folio.set_text(text)` | Replaces all of it; `\n` starts a new paragraph |
| `folio.get_cursor()` | Where the caret is: the number of characters before it, counting each paragraph break as one (0 is the start) |
| `folio.set_cursor(index)` | Moves the caret there (past the end means the end) |
| `folio.get_selection()` | The selection's start, end and text (start and end are equal, and the text empty, when nothing is selected) |
| `folio.set_selection(from, to)` | Selects from `from` to `to`, with the caret at `to`; `set_selection(i)` selects nothing and puts the caret at `i` |
| `folio.insert_text(text)` | Types `text` at the caret, replacing any selection |
| `folio.register_command(name, function, label)` | Adds a command `lua <name>` for `keys.toml` and the Scripts menu. `label` is optional: what the menu shows instead of the name |
| `folio.get_path()` | The document's file path, or `nil` if it hasn't been saved yet. Scripts still can't open it |
| `folio.notify(text)` | Shows a short message in the status line (the first line, up to 200 characters) |

Each command is one step for Undo. `print(...)` writes to **Help → Log**
(`Cmd+Alt+L`), and so does every error, with the line it happened on; a
broken script never stops Folio.

Scripts can't touch your files, run programs or use the network: they get
Lua's `string`, `table`, `math`, `utf8` and `coroutine` libraries and the clock
(`os.time`, `os.clock`, `os.date`, `os.difftime`), and nothing else. A
command that runs longer than a second is stopped, and scripts share 32 MB
of memory.

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
| `%adapter_status%` | What adapters and plugins show in the status line |
| `%plugin_status%` | Just the plugins' part |
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

[[panels]]
kind = "plugin"         # filled by a WebAssembly plugin
plugin = "Word Goals"
id = "goal"
```

Right-click a panel's title to move it to another side or close it, and drag
its edge to resize. Folio writes those changes back into settings.toml.
`Cmd+\` hides or shows all panels.

The outline lists `title`, `h1`, `h2` and `h3` paragraphs. A theme can change
that per style with `outline = <level>` (`0` leaves a style out).

## Themes

See [themes/paper.toml](../themes/paper.toml) for every option.

A font can be a path (absolute, `~/…`, or relative to the theme file), a bare
file name looked up in the system's font folders, or a list where the first
one this computer has wins. That's how the built-in themes work everywhere:

```toml
[fonts.serif]
regular = ["Georgia.ttf", "georgia.ttf", "DejaVuSerif.ttf"]   # macOS, Windows, Linux
```

Themes can also set:

```toml
[page]
header = "%file%"
footer = "%page% / %pages%"
print_background = false   # true prints the page color in PDFs

[colors]
panel = "#2a2a2a"
panel_text = "#cccccc"
find = "#ffcc0055"         # highlight behind find matches
link = "#2a63c9"           # links in the text

[lists]                    # list items (make them with the Rich Content adapter)
indent = 0.3               # inches per level
bullets = ["•", "◦", "▪"]  # by level, repeating
numbers = ["1.", "a.", "i."]  # 1. 2. 3. / a. b. c. / i. ii. iii. (also "A.", "I.", "1)")
```
