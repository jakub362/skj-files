# SKJ Files 0.4.0

A fast, good-looking file manager for **Windows, Linux and macOS** — part of the SKJ App Pack.
Written in **Rust** with a **Slint** UI. Starts in a fraction of a second.

## Features

- **Tabs** — Ctrl+T / Ctrl+W, middle-click a folder to open it in a new tab, tabs come back on next start
- **Dual pane** — two folders side by side (Tab switches pane)
- **Details and Grid views** — grid shows real image thumbnails
- **Preview pane** — images, text/code, folder contents, file info
- **Home page** — quick access, drives with usage bars, recent folders
- **Address bar** — click a part of the path, or click empty space / Ctrl+L to type a path (`~`, `%USERPROFILE%` work)
- **Search** — type to filter instantly; press Enter to search all subfolders
- **File operations** — copy / cut / paste with progress, rename, new folder / file, folder from selection,
  move to Recycle Bin / Trash, permanent delete (Shift+Del)
- **Windows clipboard interop** — copy in Explorer, paste in SKJ Files (and the other way)
- **Live updates** — folders refresh by themselves when files change
- **Properties** — sizes of whole folders, dates, attributes, SHA-256 hash
- **Pin folders** to the sidebar (right-click → Pin to sidebar)
- **Open in terminal**, Open with…, Copy path, New window
- **Dark and light theme, 7 accent colours**; the window title bar follows the theme

### New in 0.2

- **Hashes** — CRC32, MD5, SHA-1, SHA-256, SHA-512 and BLAKE3 in one pass (all computed in parallel),
  copy any of them, paste a hash to check it; several files → SHA-256 of each + "identical / different" for two files
- **Verify checksums** — open a `SHA256SUMS` / `.sha256` / `.md5` file and every listed file is checked (✓ / ✗)
- **ZIP** — Compress to ZIP (keeps file dates), Extract to a folder or Extract here (.zip, .tar, .tar.gz, .tgz, .gz) — never overwrites
- **Batch rename** — select several files, F2: find & replace and/or a pattern with `{name} {n} {nn} {nnn} {date} {ext}`, live preview
- **Undo (Ctrl+Z)** — rename, batch rename, move, copy, new folder/file, compress, extract, and Recycle Bin / Trash deletes
- **Colour tags** — right-click → Tag; tagged files get dots, the sidebar lists every tag
- **Quick look (Space)** — big preview of images and text, ← → to flip through
- **Drag & drop** — drag files onto folders, the other pane, sidebar folders or a tag (Ctrl = copy; another drive = copy)
- **Tools menu**, preview-pane quick actions, tooltips with shortcuts, three grid sizes, friendly dates
- **Git** branch badge in the address bar, **Open in VS Code**

### New in 0.3

- **Name clashes** — pasting or dropping where a name is taken asks: **Replace** (the old one goes to the bin),
  **Keep both** (" - Copy") or **Skip**, shows which one is newer / larger, and can apply your answer to all clashes.
  Folders can be **merged**
- **Cancel** long operations; copies show **speed and time left**
- **Rename in place** (F2) — the name without its extension is selected; **New folder / new file** appear right away, ready to be named
- **Selection box** — drag from empty space (or the blank part of a row) to select; scrolls when you reach the edge
- **Columns** — drag a column's edge to resize it, right-click the header to show **Date created**, Type, Size…; sort by date created
- **Recycle Bin / Trash view** inside the app — restore (with a warning when the original place is taken), delete permanently, empty
- **Real thumbnails and icons** — video frames and PDF pages in the grid, real **.exe / shortcut icons** (Windows: what Explorer shows;
  Linux: the shared thumbnail cache, `ffmpegthumbnailer`/`ffmpeg`, `pdftoppm`; macOS: Quick Look)
- **Drag & drop with other apps** — on Windows drag files **out to Explorer**, the desktop or any app, and **in from Explorer** onto a folder,
  the pane or the sidebar (real OLE drag & drop with Explorer's copy/move rules). macOS and Linux (X11) accept drops into the current folder
- Settings are written safely (temp file + backup)

### New in 0.4

- **Command palette (Ctrl+Shift+P)** — every command, folder, drive, recent folder and open tab in one search box;
  type a path to jump there. Shows each command's shortcut
- **Column view** (Miller columns, like macOS Finder) — one column per folder level; ← / → move between levels
- **Tabs** — drag to reorder, right-click for Duplicate / Move to a new window / Close others / Close to the right;
  **Ctrl+Shift+T** reopens a closed tab; drop files on a tab to move them into that tab's folder
- **Windows menu** — right-click → **Show more options** (or Shift+F10) opens Explorer's own context menu
  (7-Zip, Git, antivirus… entries all work)
- **Network** — **Connect to server** (`\\server\share`, `smb://`, `sftp://`), saved servers in the sidebar's Network section
- **Recent files** on the Home page (yours and the ones the system remembers)
- **Custom shortcuts** — Settings → Shortcuts: click a command, press the new keys (Backspace resets)
- **Custom toolbar** — Settings → Toolbar: pick which buttons are shown
- **Polish language** — Settings → System → Language: Automatic (follows the system), English or Polski. Switches instantly,
  including dates (Dziś, Wczoraj, weekdays), numbers (1 234,5 MB) and Polish plural forms (1 plik, 2 pliki, 5 plików)
- **Default file manager** — Settings → System: folders, drives and **Win+E** open in SKJ Files instead of File Explorer
  (Linux: becomes the default for folders). Turning it off puts everything back

## Keyboard

| Keys | Action |
| --- | --- |
| Enter / double-click | Open |
| Backspace, Alt+← / Alt+→ | Back / forward |
| Alt+↑ | Up one folder |
| ↑ ↓ (← → in grid), Home, End, PgUp, PgDn | Move (hold Shift to select) |
| Letters | Jump to a file starting with them |
| Ctrl+A / Esc | Select all / clear |
| Ctrl+C / Ctrl+X / Ctrl+V | Copy / cut / paste |
| Ctrl+Shift+C | Copy path |
| F2 | Rename (in place; several files → batch rename) |
| Del / Shift+Del | Recycle Bin / delete permanently (in the bin view: delete permanently) |
| Ctrl+Shift+N | New folder |
| Ctrl+T / Ctrl+W / Ctrl+Tab / Ctrl+1…9 | Tabs |
| Ctrl+L or Alt+D or F4 | Type a path |
| Ctrl+F or F3 | Search |
| Ctrl+H | Show hidden files |
| Ctrl+P | Preview pane |
| F5 | Refresh |
| Alt+Enter | Properties |
| Ctrl+N | New window |
| Ctrl+, | Settings |
| Space | Quick look |
| Ctrl+Z | Undo |
| Ctrl+Shift+H | Hashes |
| F2 with several files | Batch rename |
| Ctrl+Shift+P | Command palette |
| Ctrl+Shift+T | Reopen closed tab |
| Shift+F10 | Windows "Show more options" menu (Windows) |

All of these can be changed in Settings → Shortcuts.

## Build

Fedora:

```bash
sudo dnf install -y rust cargo gcc fontconfig-devel libxkbcommon-devel wayland-devel mesa-libGL-devel
cargo build --release
./target/release/skj-files            # or: ./target/release/skj-files ~/Pobrane
```

Windows (native): install Rust from rustup.rs, then `cargo build --release`.

Cross-compile the Windows .exe from Linux:

```bash
rustup target add x86_64-pc-windows-gnu
sudo apt install gcc-mingw-w64-x86-64     # Fedora: sudo dnf install mingw64-gcc
cargo build --release --target x86_64-pc-windows-gnu
```

macOS (universal app, built from Linux): see `scripts/package-macos.sh` — it needs a macOS SDK
(`SDKROOT`), `cargo-zigbuild` and **zig 0.16** (zig 0.17 writes duplicate framework entries that stop the app from starting).

Set `SKJ_TIMING=1` to print startup timings.

## Installing on macOS

Unzip `SKJ-Files-<version>-macos-universal.zip` and drag **SKJ Files.app** into Applications.
It runs natively on Apple Silicon and Intel Macs (macOS 11+). The app isn't signed with an Apple
Developer ID yet, so before the first launch run this once in Terminal:

```bash
xattr -cr "/Applications/SKJ Files.app"
```

(or try opening it, then System Settings → Privacy & Security → **Open Anyway**).

## Notes

- `vendor/i-slint-backend-winit` is Slint's winit backend with two SKJ changes: a monitor reporting a 0 Hz refresh
  rate (some remote/virtual displays) no longer crashes the app, and on Windows it does native OLE drag and drop
  (`win_dnd.rs`) so files can be dragged to and from Explorer. Set `SKJ_DND_DEBUG=1` to log drag & drop.
- **Translations**: Rust strings are English in the code and translated through `src/i18n_pl.rs`
  (`ts("…")`, `tf!("…", args)`; Polish templates can use `{0|plik|pliki|plików}` for plural forms).
  The Slint UI uses `@tr("…")`; its Polish strings are in `scripts/slint_pl.py` and `python3 scripts/make-po.py`
  writes `lang/pl/LC_MESSAGES/skj-files.po` (bundled into the binary) and fails if a string has no translation.
- Settings live in the config folder (`%APPDATA%\skj-files\settings.ini` on Windows, `~/.config/skj-files/` on Linux).

## Licences

SKJ Files © SKJ Tech. Uses [Slint](https://slint.dev) under the Slint Royalty-free Licence (AboutSlint shown in Settings),
[Lucide](https://lucide.dev) icons (ISC) and the [Inter](https://rsms.me/inter/) font (SIL OFL) — licence files are in `ui/icons` and `ui/fonts`.
