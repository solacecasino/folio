<p align="center"><img src="assets/icon.png" width="128" alt="text9000 icon"></p>

<h1 align="center">text9000</h1>

<p align="center"><b>Bare when installed, insanely customizable.</b></p>

text9000 is a lean document editor for macOS, Windows and Linux, written in Rust. It opens to
a blank page and a cursor: no ribbon, no template store, no account, nothing
running in the background. How your documents look comes from plain theme
files you own, and anything extra comes from add-ons you choose, in the
spirit of [foobar2000](https://www.foobar2000.org/).

This page is the user guide. Read it once and you'll know everything text9000
can do; it takes about ten minutes. text9000's **Help → User Guide** opens this
page.

**Contents:** [Install](#install) · [Your first document](#your-first-document) ·
[Writing and formatting](#writing-and-formatting) · [Saving, history and recovery](#saving-history-and-recovery) ·
[Pages, printing and PDF](#pages-printing-and-pdf) · [Add-ons](#add-ons) ·
[Making text9000 yours](#making-text9000-yours) · [Shortcuts](#shortcuts) ·
[Help and problems](#help-and-problems) · [The 9000 series](#the-9000-series) ·
[Coming from Folio](#coming-from-folio) · [Build from source](#build-from-source)

---

## Install

Download yours from [Releases](../../releases/latest):

| | Download | Then |
|---|---|---|
| **macOS** (Apple Silicon and Intel) | `text9000-<version>-macos-universal.zip` | Unzip and drag **text9000.app** into Applications. It isn't signed with an Apple developer certificate yet, so the first time, right-click it and choose **Open**. |
| **Windows** (64-bit) | `text9000-<version>-windows-x64.zip` | Unzip anywhere and run **text9000.exe** (keep the `themes` folder next to it). If Windows says it protected your PC, click **More info → Run anyway**. |
| **Linux** (x86-64) | `text9000-<version>-linux-x64.tar.gz` | Unpack it and run `./install.sh` to add text9000 to your app menu, or just run `./text9000`. You need `zenity` for open and save dialogs. |

Each also comes as **text9000 Lite** (`text9000-lite-…`): the same
app without WebAssembly plugins, about 40% smaller. Adapters work in both.

> **Windows and Linux:** the menus are inside the window, and every `Cmd`
> in this guide means `Ctrl`.

---

## Your first document

Open text9000 and start typing. That's the whole interface: a page, a cursor,
and a status line along the bottom with your word count.

- **Save** with `Cmd+S`. text9000 documents end in `.t9k`. Documents from Folio (`.folio`) open as they are and save back as `.folio`.
- **Open** with `Cmd+O`, or **File → Open Recent**, or drop a `.t9k` or `.folio` file on the window. Videos, sound and photos open in their [9000 apps](#the-9000-series).
- **New** with `Cmd+N`.
- **See every shortcut** with `Cmd+/`. Keep this one in mind; it's the cheat sheet.

text9000 remembers its window size and place, and the documents you opened
recently. There's no settings window; [Making text9000 yours](#making-text9000-yours)
explains why, and how to change anything.

---

## Writing and formatting

**Text.** `Cmd+B`, `Cmd+I` and `Cmd+U` for bold, italic and underline, or
the **Format** menu. As you type, text9000 turns straight quotes into curly
ones, `--` into an em dash, and `...` into an ellipsis.

**Paragraph styles.** Title, headings, body, quote and the rest come from
the theme. Apply one from the **Format** menu or with `Cmd+0` to `Cmd+9`
(the theme decides which number is which; `Cmd+/` shows them). Pressing Enter
after a heading goes back to body text by itself.

**Tab** indents. In a list, `Tab` and `Shift+Tab` move an item one level in
or out.

**Lists and links** need the free **Rich Content** add-on (see
[Add-ons](#add-ons)). With it:

| | |
|---|---|
| `Cmd+Shift+8` | Bulleted list |
| `Cmd+Shift+7` | Numbered list |
| `Cmd+K` | Add a link to the selected text |
| **Format → Remove Link** | Remove one |

Press Enter on an empty list item to end the list. `Cmd`-click a link to
open it. Documents with lists and links still open and look right without
the add-on; you just can't make new ones.

**Find and replace.** `Cmd+F` finds, `Cmd+G` and `Cmd+Shift+G` go to the next
and previous match, and `Cmd+Alt+F` finds and replaces. Both can match case
and whole words.

**Spell check** uses your computer's own dictionaries. Misspelled words are
underlined; right-click one for suggestions.

**Copy and paste** between text9000 documents keeps formatting. Pasting from
other apps brings in plain text.

**Undo** is `Cmd+Z`, **redo** is `Cmd+Shift+Z`.

---

## Saving, history and recovery

text9000 is built so you don't lose work:

- **Autosave.** Once a document has a name, text9000 saves it two seconds after you stop typing.
- **Version history.** Every time you save, and every five minutes while autosave is working, text9000 keeps a snapshot (the last 50 per document). **File → Browse Versions…** shows them; click one to restore it. The restore can itself be undone.
- **Crash recovery.** Work you haven't named yet is kept in a recovery copy. If text9000 or your computer crashes, text9000 offers it back the next time it opens.

Your documents are readable JSON files on your own disk. Nothing is sent anywhere.

---

## Pages, printing and PDF

text9000 shows real pages, with the page size, margins, headers and footers
your theme sets, and long paragraphs flow across pages as they will print.
Zoom with `Cmd+=` and `Cmd+-`.

To make a PDF, add the **PDF Export** add-on, then use **File → Export as
PDF** (`Cmd+E`). The PDF matches the screen exactly, links included.

---

## Add-ons

text9000 comes with nothing extra on purpose. Anything optional is an add-on
you choose. Some good ones to start with:

| | |
|---|---|
| **Rich Content** | Bulleted and numbered lists, and links |
| **PDF Export** | Print-perfect PDFs of your documents |
| **Folio Link** | Write together live with people you invite, peer to peer, no server or account |
| **Writing Tools** | A word-count goal and document stats |

**To install one:**

1. Open the [add-on catalog](https://github.com/solacecasino/folio-adapters/blob/main/CATALOG.md).
2. Download the zip for your computer.
3. Drop the zip on the text9000 window.

That's it: no restart. text9000 shows you what the add-on is allowed to do (for
example "read your document") and asks before it runs the first time.
**text9000 → Adapters…** (`Cmd+Shift+A`) lists everything you've added, runs its
commands, and removes it. text9000 never goes online to do any of this; you do
the downloading.

**Inviting someone with Folio Link:** choose **Share This Document** from
its commands, and text9000 copies an invite link (`folio://join#…`). Send it to them;
when they click it, text9000 opens with your document and you see each other's
typing.

There are two kinds of add-on. **Plugins** are small WebAssembly programs
that run in a sandbox and can only touch what they declared; they appear in a
**Plugins** menu (not in text9000 Lite). **Adapters** are separate programs, in
any language, for connecting outside tools. You install both the same way.
[How they differ](docs/plugins.md).

**Make your own:** start from the
[adapter template](https://github.com/solacecasino/folio-adapter-template)
(a working Python adapter in about 15 lines) or the
[plugin template](https://github.com/solacecasino/folio-plugin-template)
(Rust). Each comes with a workflow that publishes downloads for every
platform when you push a version tag, and listing it in the catalog is a
one-field form. Guides: [docs/adapters.md](docs/adapters.md),
[docs/plugins.md](docs/plugins.md). **Help → Log** (`Cmd+Alt+L`) shows what
your add-on prints while you build it.

---

## Making text9000 yours

text9000 has no settings window. Everything is a plain text file in one folder,
which you open with **text9000 → Open Config Folder**:

| | |
|---|---|
| macOS | `~/Library/Application Support/text9000` |
| Windows | `%APPDATA%\text9000` |
| Linux | `~/.config/text9000` |

In it:

- **`settings.toml`**: theme, zoom, the status line, panels, autosave and history. It explains every option inside itself.
- **`keys.toml`**: every shortcut, all changeable.
- **`init.lua`** (optional): your own commands, written in Lua. They appear in a **Scripts** menu and can be bound to keys. text9000 puts an `init.lua.example` in the folder; rename it to `init.lua` to try it. See [docs/customizing.md](docs/customizing.md#initlua).
- **`themes/`**: your own themes.
- **`adapters/`** and **`plugins/`**: the add-ons you've installed.

Save a file and text9000 changes right away; no restart. Copy the folder to
another computer, or share it, and you've moved your whole setup.

### Themes

A theme is one file that sets the page size and margins, colors, fonts,
headers and footers, and every paragraph style. Three come with text9000:
**Paper**, **Modern** and **Manuscript**. `Cmd+T` switches between them.

To make your own, copy [themes/paper.toml](themes/paper.toml) (it lists every
option) into the `themes` folder of your config folder, change its `name`,
and edit away:

```toml
name = "My Theme"

[page]                 # inches
width = 8.5
height = 11
margin_left = 1.1

[colors]
background = "#3a3a3a"
page = "#fdfcf8"

[fonts.serif]          # any font on your computer; the first one found wins
regular = ["Georgia.ttf", "DejaVuSerif.ttf"]

[styles.h1]
font = "serif"
size = 20
bold = true
key = 1                # Cmd+1 applies it
next = "body"          # the style after pressing Enter
```

### Shortcuts

In `keys.toml`, map a key to any command, or to `"none"` to turn it off:

```toml
"cmd+shift+h" = "style h2"
"cmd+t" = "none"
```

`Cmd+/` lists every command's name.

### Status line, title and panels

The status line, window title, page headers and footers are written in a
small formatting language:

```toml
# settings.toml
status = "%words% $plural(%words%,word)   page %page%/%pages%"
```

Square brackets show a part only when it has something in it:
`[ · %selection_words% selected]` appears only while you have a selection.

**Panels** are off until you add them. An outline of your headings, your own
stats, earlier versions, or a panel from an add-on, on any side:

```toml
[[panels]]
kind = "outline"
side = "left"
```

The full reference is [docs/customizing.md](docs/customizing.md).

---

## Shortcuts

| | |
|---|---|
| `Cmd+N` / `Cmd+O` / `Cmd+S` | New, open, save |
| `Cmd+Shift+S` | Save as |
| `Cmd+Z` / `Cmd+Shift+Z` | Undo, redo |
| `Cmd+F` / `Cmd+G` / `Cmd+Alt+F` | Find, find next, find and replace |
| `Cmd+B` / `Cmd+I` / `Cmd+U` | Bold, italic, underline |
| `Cmd+0`–`Cmd+9` | Paragraph styles (set by the theme) |
| `Tab` / `Shift+Tab` | Indent; in a list, one level in or out |
| `Cmd+Shift+8` / `Cmd+Shift+7` / `Cmd+K` | Bullets, numbers, link (Rich Content add-on) |
| `Cmd`-click | Open a link |
| `Cmd+E` | Export as PDF (PDF Export add-on) |
| `Cmd+T` | Next theme |
| `Cmd+=` / `Cmd+-` | Zoom |
| `Cmd+.` / `Cmd+\` | Show or hide the status line / panels |
| `Cmd+Shift+A` | Add-ons (Adapters window) |
| `Cmd+Alt+L` | Log |
| `Cmd+Shift+R` | Reload config |
| `Cmd+/` | Every shortcut |

---

## Help and problems

- **Something wrong?** Choose **Help → Report a Problem…**. text9000 copies its
  debug info and opens the [problem report](../../issues/new?template=bug.yml)
  form in your browser; paste it in. The debug info says which version and
  platform you're on, how much memory text9000 is using (and the most it has
  used), the window's size and zoom, and what went wrong recently. It never
  includes your document, and your user name is hidden from file paths.
  **Help → Log → Copy Debug Info** copies the same thing without opening the
  form.
- **text9000 crashed?** It keeps the details, and tells you the next time it
  opens. Report a Problem then includes them.
- **Feels slow?** Start text9000 from a terminal with `TEXT9000_FRAME_TIMES=1`
  (on Windows: `set TEXT9000_FRAME_TIMES=1` first) and it prints, for every
  frame, how much of the window it redrew and how long that took. Paste a
  few lines into your report.
- **An idea?** [Suggest it](../../issues/new?template=idea.yml).
- **A problem with an add-on?** Report it on that add-on's own page, linked
  from the catalog.

**Coming next:** images and tables, then a gallery of shared themes and
setups.

---

## The 9000 series

text9000 is one of four apps built the same way: bare, modular, configured
with plain files. Its sisters are
[splice9000](https://github.com/solacecasino/splice9000) (video),
[wave9000](https://github.com/solacecasino/wave9000) (sound) and
[grace9000](https://github.com/solacecasino/grace9000) (photos). text9000
works with them directly:

- **Open or drop** a video, a sound file or a photo, or a `.splice`, `.wave`
  or `.grace` project, and it opens in the right sister app instead.
- **Link to them.** **Format → Link to a File…** links the selection (or the
  file's name) to any file; `Cmd`-click the link and the file opens in its
  9000 app. A script or shot list can point at the edit, the takes and the
  stills. Links to files in the document's folder (or below it) are written
  relative to it, so the folder can be moved or shared as a whole.
- **The 9000 menu** opens splice9000, wave9000 or grace9000. It shows only
  the ones that are installed. Bind one to a key with `app splice9000` in
  `keys.toml`.

text9000 finds the sister apps by itself: on macOS wherever they are, on
Linux through their `install.sh` menu entries or your `PATH`, and on Windows
on your `PATH` or unzipped next to text9000's own folder. If it doesn't find
one, name it in `settings.toml`:

```toml
[apps]
splice9000 = "C:/Apps/splice9000/splice9000.exe"
```

All four share one look: graphite with one signal color each (splice orange,
wave blue, grace violet, and text9000's ink green), the name with 9000 in
mono, and a mark cut on the same slant.

## Coming from Folio

text9000 *is* Folio, renamed when it joined the 9000 series, at version 3.0.
Nothing you had stops working:

- **Your setup comes along.** The first time text9000 starts, it copies
  Folio's whole config folder (settings, keys, themes, `init.lua`, adapters,
  versions) into its own. Folio's folder is left as it was.
- **`.folio` documents open** and save back as `.folio`; new documents are `.t9k`.
  The file format inside is the same, so Folio 2 can open a `.t9k` renamed to `.folio`.
- **`folio://` links**, such as Folio Link invites, still work.
- **Scripts:** the `folio` table in `init.lua` is still there; `text9000` is a
  second name for the same table.
- **Adapters and plugins** written for Folio work unchanged.
- **`FOLIO_CONFIG_DIR`** and the other `FOLIO_…` variables still work;
  `TEXT9000_…` is the new spelling.
- On Linux, `install.sh` replaces Folio's menu entry and leaves `folio` in
  `~/.local/bin` as a shortcut to text9000.

What did change: the app's name, icon and window colors, the config folder's
name, and the macOS bundle (`app.text9000.editor`), so macOS asks once more
before opening it, and the Linux window class (`text9000`).

---

## Build from source

You need [Rust](https://rustup.rs/).

```sh
cargo run --release -- examples/welcome.folio   # run it on the sample document
python3 scripts/package.py                      # build this platform's download into dist/
```

On Linux you also need the usual windowing libraries, for example on Debian
or Ubuntu: `sudo apt install libxkbcommon-dev`. Releases are
built on all three platforms by [GitHub Actions](.github/workflows/release.yml)
when a version tag is pushed. How text9000 is built, and why:
[ARCHITECTURE.md](ARCHITECTURE.md).

## License

[MIT](LICENSE)
