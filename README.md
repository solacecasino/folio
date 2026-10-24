<p align="center"><img src="assets/icon.png" width="128" alt="Folio icon"></p>

<h1 align="center">Folio</h1>

<p align="center"><b>Bare when installed, insanely customizable.</b></p>

Folio is a lean document editor for macOS, written in Rust. It starts as a
blank page and a cursor: no ribbon, no template store, no account. Everything
about how documents look comes from plain theme files you own, and everything
extra comes from optional **adapters** you choose to install, in the spirit
of [foobar2000](https://www.foobar2000.org/).

## Features

- **Real pages:** page size, margins and pagination, like a word processor; long paragraphs flow across pages
- **A safety net:** autosave, crash recovery, and a version history you can browse and restore
- **Spell check** with the macOS dictionaries, plus smart quotes, em dashes and ellipses as you type
- **Find & replace** with match case and whole words
- **Rich text:** bold, italic, underline and paragraph styles (title, headings, quote, …)
- **Themes are files:** fonts, colors, page setup, headers and footers, and every paragraph style live in a TOML file. Copy one, change anything, share it. No paid templates.
- **No settings window:** `settings.toml` and `keys.toml` control everything, reload live as you save them, and every shortcut is remappable
- **A formatting language** for the status line, window title, page headers and footers, and panels: `%words% $plural(%words%,word)[ · %selection_words% selected]`
- **Panels you arrange:** an outline, your own stats panels, and adapter panels on any side, all off until you add them
- **Adapters:** everything optional is a separate add-on you choose, in any language, approved by you and started only when used
- **Your files:** `.folio` documents are readable JSON on your own disk
- **Small and fast:** native, no Electron, nothing running in the background

## Install

Download `Folio.app` from [Releases](../../releases) and drag it into Applications.

The app isn't signed with an Apple developer certificate yet, so the first time
you open it, right-click it and choose **Open**.

## Build from source

You need [Rust](https://rustup.rs/).

```sh
cargo run --release -- examples/welcome.folio   # run it
./scripts/bundle.sh                              # build target/Folio.app
```

## Customizing

Everything lives in `~/Library/Application Support/Folio/` (**Folio → Open
Config Folder**): `settings.toml`, `keys.toml`, `themes/`, and the
`adapters/` you've added.
Share that folder and you've shared your whole setup. The full guide is in
[docs/customizing.md](docs/customizing.md).

```toml
# keys.toml
"cmd+shift+h" = "style h2"
"cmd+t" = "none"

# settings.toml
status = "%words% $plural(%words%,word)   page %page%/%pages%"

[[panels]]
kind = "outline"
side = "left"
```

## Themes

Themes live in `themes/` and in `~/Library/Application Support/Folio/themes`.
Three come with Folio: **Paper**, **Modern** and **Manuscript**. `Cmd+T` cycles
through them. A theme sets:

```toml
name = "Paper"

[page]                 # inches
width = 8.5
height = 11
margin_left = 1.1

[colors]
background = "#3a3a3a"
page = "#fdfcf8"

[fonts.serif]          # any .ttf on your machine
regular = "/System/Library/Fonts/Supplemental/Georgia.ttf"

[styles.h1]
font = "serif"
size = 20
bold = true
key = 1                # Cmd+1 applies it
next = "body"          # style after pressing Enter
```

See [themes/paper.toml](themes/paper.toml) for every option.

## Adapters

Folio.app ships with nothing extra. Anything optional is an **adapter**, a
small add-on you install only if you want it. Adapters live in their own repo,
[**folio-adapters**](https://github.com/solacecasino/folio-adapters):

| Adapter | Adds |
|---|---|
| pdf-export | **Export as PDF** (`Cmd+E`): print-perfect, matching the page line for line, with your theme's fonts embedded and searchable text |
| folio-link | **Live co-editing** with people you invite, over encrypted peer-to-peer links. No accounts, no servers of ours. |
| writing-tools | A word-count goal panel, today's date, document stats (about 100 lines of Python, a good starting point) |

To add one, download its zip from the
[folio-adapters releases](https://github.com/solacecasino/folio-adapters/releases).
Then in Folio choose **Folio → Adapters…** (`Cmd+Shift+A`), click **Add
Adapter…**, and pick the zip. It works right away, with no restart. The same
window shows what each adapter can do, runs its commands, and removes it.

Adapters can be written in any language. The first time one runs, Folio shows
what it asks for and you decide. The protocol is in
[docs/adapters.md](docs/adapters.md).

## Shortcuts

| | |
|---|---|
| `Cmd+N` / `Cmd+O` / `Cmd+S` | New, open, save |
| `Cmd+Shift+A` | Adapters window |
| `Cmd+E` | Export as PDF (with the pdf-export adapter) |
| `Cmd+F` / `Cmd+G` / `Cmd+Alt+F` | Find, find next, find and replace |
| `Cmd+B` / `Cmd+I` / `Cmd+U` | Bold, italic, underline |
| `Cmd+0`–`Cmd+9` | Paragraph styles (set by the theme) |
| `Cmd+T` | Next theme |
| `Cmd+=` / `Cmd+-` | Zoom |
| `Cmd+.` / `Cmd+\` | Show or hide the status line / panels |
| `Cmd+Shift+R` | Reload config |
| `Cmd+/` | All shortcuts (every one can be changed in `keys.toml`) |

## Roadmap

- Lists, links and images
- Lua scripting for small tweaks
- Shareable setups and a theme gallery

## License

[MIT](LICENSE)
