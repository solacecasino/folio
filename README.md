<p align="center"><img src="assets/icon.png" width="128" alt="Folio icon"></p>

<h1 align="center">Folio</h1>

<p align="center"><b>Bare when installed, insanely customizable.</b></p>

Folio is a lean document editor for macOS, Windows and Linux, written in Rust. It starts as a
blank page and a cursor: no ribbon, no template store, no account. Everything
about how documents look comes from plain theme files you own, and everything
extra comes from optional **adapters** you choose to install, in the spirit
of [foobar2000](https://www.foobar2000.org/).

## Features

- **Real pages:** page size, margins and pagination, like a word processor; long paragraphs flow across pages
- **A safety net:** autosave, crash recovery, and a version history you can browse and restore
- **Spell check** with your system's dictionaries, plus smart quotes, em dashes and ellipses as you type
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

Download yours from [Releases](../../releases):

| | Download | Then |
|---|---|---|
| **macOS** (Apple Silicon) | `Folio-<version>-macos-arm64.zip` | Unzip and drag **Folio.app** into Applications. It isn't signed with an Apple developer certificate yet, so the first time, right-click it and choose **Open**. |
| **Windows** (64-bit) | `Folio-<version>-windows-x64.zip` | Unzip anywhere and run **folio.exe** (keep the `themes` folder next to it). If Windows says it protected your PC, click **More info → Run anyway**. |
| **Linux** (x86-64) | `folio-<version>-linux-x64.tar.gz` | Unpack it and run `./install.sh` to add Folio to your app menu, or just run `./folio`. |

On Windows and Linux the menus are in the window, and every `Cmd` shortcut
below is `Ctrl`.

## Build from source

You need [Rust](https://rustup.rs/).

```sh
cargo run --release -- examples/welcome.folio   # run it
python3 scripts/package.py                      # build this platform's download into dist/
```

On Linux you also need the usual windowing libraries, e.g. on Debian or
Ubuntu: `sudo apt install libxkbcommon-dev libgl1-mesa-dev`. Releases are
built on all three platforms by [GitHub Actions](.github/workflows/release.yml)
when a version tag is pushed.

## Customizing

Everything lives in one folder (**Folio → Open Config Folder**):
`~/Library/Application Support/Folio` on macOS, `%APPDATA%\Folio` on Windows,
`~/.config/folio` on Linux. In it are `settings.toml`, `keys.toml`, `themes/`, and the
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

Themes live in `themes/` and in the `themes` folder of your config folder.
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

[fonts.serif]          # any .ttf on your machine: a path, a file name, or a list
regular = ["Georgia.ttf", "georgia.ttf", "DejaVuSerif.ttf"]   # first one found wins

[styles.h1]
font = "serif"
size = 20
bold = true
key = 1                # Cmd+1 applies it
next = "body"          # style after pressing Enter
```

See [themes/paper.toml](themes/paper.toml) for every option.

## Adapters

Folio ships with nothing extra. Anything optional is an **adapter**, a
small add-on you install only if you want it.

**Find them in the [adapter catalog](https://github.com/solacecasino/folio-adapters/blob/main/CATALOG.md)**,
which has direct downloads for macOS, Windows and Linux. Download the zip for
your computer and **drop it on the Folio window**. That's it: no restart, and
Folio shows what the adapter can do and asks before it first runs. **Folio →
Adapters…** (`Cmd+Shift+A`) lists what you've added, runs its commands, and
removes it. Folio itself never goes online to do any of this.

Some to start with: **PDF Export** (print-perfect PDFs), **Folio Link** (live
co-editing with people you invite, peer to peer) and **Writing Tools** (a
word-count goal and document stats).

**Make your own** in any language. Start from
[folio-adapter-template](https://github.com/solacecasino/folio-adapter-template):
a working Python adapter in about 15 lines, and a workflow that publishes it
for every platform when you push a version tag. Listing it in the catalog is
a form with one field. The protocol is in [docs/adapters.md](docs/adapters.md).

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
