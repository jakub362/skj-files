# SKJ Files — notes for Claude Code

Fast, good-looking file manager for Windows, Linux and macOS. **Rust + Slint 1.18** (fluent style).
First app of the **SKJ App Pack** (SKJ Tech, owner: Jakub). Later the pack gets a hub app (SKJ Hub),
and SKJ Archiver is merged into Files only after the whole pack is finished — not now.

Current version: **1.0.0**. Releases are built by GitHub Actions from a `v*` tag, for Windows
(installer + portable), Linux (binary + Flatpak) and macOS (universal .dmg).
Earlier work (0.1–0.4) was done in a cloud session; this file carries what that session knew.

## How Jakub works (follow these)

- He writes fast with typos; read for intent. He's Polish; the app has a Polish UI.
- Prefers **one big code block to paste** over many separate commands.
- **Ask before** creating or pushing any GitHub repo, and before changing the licence
  (**GPL-3.0-only** since 2026-10-28, chosen because free SignPath code signing needs it;
  the repo is public at github.com/jakub362/skj-files).
- **Keep the Notion project log updated without being asked** (if the Notion MCP is connected):
  SKJ Files page https://app.notion.com/p/40645efc42ae81518bded47e5dc547fe in the "SKJ Project Log" database
  (data source `collection://002d30fb-7fa1-4e40-9613-2fd55b1e0bb6`). After each chunk of work: add a dated
  work-log section, an "Errors hit and fixes" section, tick the roadmap checklist, update the
  "Current state" and "Last worked on" properties. App ideas go to the "Daily App Ideas" database
  (`collection://cf760dab-e94b-4700-96bc-8cbac934a8b1`) — append only, never edit/delete entries.
- On his Fedora PC projects live in `~/Pobrane`; don't suggest moving them.

## Build

```
cargo build --release                                   # native (Windows: MSVC toolchain, needs VS Build Tools C++)
cargo build --release --target x86_64-pc-windows-gnu    # Windows .exe cross-built from Linux (mingw, see .cargo/config.toml)
SDKROOT=/path/MacOSX14.5.sdk bash scripts/package-macos.sh   # macOS universal .app zip, from Linux
```

- Version lives only in `Cargo.toml` (build.rs and the About box read `CARGO_PKG_VERSION`).
- macOS cross-build: `cargo-zigbuild` with **zig pinned to 0.16.0** (`pip install "ziglang==0.16.0"`) —
  zig 0.17 writes duplicate LC_LOAD_DYLIB entries and dyld refuses to start the app. Needs a macOS SDK folder.
  Can't be done from Windows; plan is GitHub Actions with a macOS runner (0.5–0.9).
- A release build takes ~3–6 min in the cloud container; **~15 min cold on kuby-laptop** (native MSVC),
  ~6 min for a relink after a `build.rs`-only change. The native MSVC build needs no code changes —
  it compiled clean first try on 2026-10-28, warnings included.
- `scripts/*.py` need Python (3.15 on kuby-laptop). `make-po.py` writes the `.po` with an explicit
  `newline='\n'` so Windows doesn't produce CRLF and fight `.gitattributes`.
- Deliverable names: `SKJ-Files-<ver>-windows-x64.exe`,
  `SKJ-Files-<ver>-linux-x64`, `SKJ-Files-<ver>-macos-universal.zip`, `skj-files-<ver>-source.zip`.
- `vendor/i-slint-backend-winit` is patched in via `[patch.crates-io]`: (1) a monitor reporting 0 Hz no
  longer divides by zero, (2) `win_dnd.rs` = native OLE drag & drop on Windows (IDropTarget/IDropSource,
  `SHDoDragDrop` with a fallback to a minimal CF_HDROP data object + `DoDragDrop`; repaints itself during
  the modal loop; reports Move back as COPY so Explorer never deletes originals). Keep the patch when
  upgrading Slint.

## Code map

- `src/main.rs` — startup: settings → `AppWindow::new()` → `app::apply_language` → `app::wire` → `App::start`.
  Cleans Explorer's odd argv (`C:"` → `C:\`).
- `src/app.rs` (~5300 lines) — all app state and UI logic. `thread_local APP`; call into it with
  `with_app(|a| …)` (defers re-entrant calls) or `with_app_ret`. Worker threads report back with
  `slint::invoke_from_event_loop`. Models that must survive (rows, tabs, tab drags) are updated **in place**
  — replacing a model cancels drags/animations. `action(&str)` is the single command dispatcher
  (prefixes `goto:`, `tab:`, `tb:`, `col:`, `sort:`, `tag:`, `grid:`, `view:`, `lang:`).
  Constants at the top: `TOOLBAR`, `DEFAULT_KEYS`, `REBINDABLE`, `TAG_NAMES`.
- `src/fsx.rs` — file system: listing, kinds/type names, formatting (sizes, dates, counts), drives,
  copy engine v2 (`run_op`, conflicts Replace/KeepBoth/Skip, merge, cancel, speed/ETA), zip/tar,
  hashes, bin view, network (`connect_server`), recent files.
- `src/thumbs.rs` — thumbnail pool (3 workers). Windows: IShellItemImageFactory (all-black guard);
  Linux: thumbnail cache → ffmpegthumbnailer/ffmpeg, pdftoppm, magick; macOS: qlmanage.
- `src/sysint.rs` — "default file manager". Windows: HKCU `Software\Classes\{Directory,Drive}\shell\skjfiles`
  + `CLSID\{52205fd8-…}` / `{20D04FE0-…}` `shell\opennewwindow\command` (Win+E) with `DelegateExecute=""`;
  off = delete them. Linux: `skj-files.desktop` + `xdg-mime`, restores the previous one. macOS: unsupported.
- `src/winshell.rs` — Windows "Show more options": IContextMenu(2/3) popup via an 80 ms thread timer
  (so our own popup is gone first), subclass forwards WM_INITMENUPOPUP/DRAWITEM/MEASUREITEM/MENUCHAR.
- `src/settings.rs` — `settings.ini` in the config dir; atomic save (tmp + .bak + rename).
- `src/i18n.rs`, `src/i18n_pl.rs` — translations (below).
- `ui/app.slint` (~4200 lines) — whole UI. Global `S` = properties/callbacks shared with Rust,
  `Theme`, `K.icon(kind)` maps int kinds to icons (64 = languages, 65 = keyboard are the newest).
  Icons are Lucide SVGs in `ui/icons`, font Inter in `ui/fonts`.

## Translations (English source, Polish UI)

- Rust: English text is the key. `ts("Copy")` → `&'static str`; `tf!("Moved {} items", n)` = translated
  `format!` (plain `{}` only); `t(&s)` for dynamic text. `mi()`, `chk()`, palette `p()`, `toast()` and
  `open_dialog()` translate their labels themselves. Polish templates may use `{0}`, `{1}` to reorder/drop
  args and `{0|plik|pliki|plików}` for Polish plural forms (1 / 2–4 / 5+). Add every new string to
  `src/i18n_pl.rs`; `python3 scripts/i18n-check.py` lists missing ones.
- Never translate: stored settings/shortcut combos, registry value names, action ids, file-system paths.
- Slint: wrap UI text in `@tr("…")` (literal `{x}` must be written `{{x}}`; plurals
  `@tr("{n} clash" | "{n} clashes" % count)`). Polish strings live in `scripts/slint_pl.py`;
  `python3 scripts/make-po.py` regenerates `lang/pl/LC_MESSAGES/skj-files.po` and fails on missing strings.
  build.rs bundles it (`with_bundled_translations("lang")`, no translation context).
- Language switch at runtime: `App::set_language` → `apply_language` (Rust + `slint::select_bundled_translation`)
  then re-syncs everything Rust wrote into the UI. Setting `lang` = auto / en / pl.
- Polish formatting: dates `28.10.2026 14:05`, Dziś/Wczoraj/weekday names, `1 234`, `13,7 GB`, „quotes”.

## Slint gotchas hit so far

- `row`, `col` are reserved property names (layout); `&&`/`||` mixed need parentheses.
- Redraws are timer-throttled: inside a Win32 modal loop call `winit_window.request_redraw()` yourself.
- Drag only starts if the drag payload is non-empty at press time (keep a placeholder).
- `select_bundled_translation` must run after the first component is created.

## Debug / test

- Env vars: `SKJ_TIMING=1` (startup timings), `SKJ_DND_DEBUG=1` (drag & drop log), `SKJ_THUMB_DUMP=<dir>`.
- On Windows just run `target\release\skj-files.exe [folder]`. On Linux headless testing used
  Xvfb + xdotool + ImageMagick `import` screenshots (keys need the window focused; beware a second
  window with the same title).
- Tested on real Windows 11 (build 26300) on 2026-10-28: Polish auto-detection, "Show more options",
  drag **into** the app from Explorer, delete to the bin, and shell icons/thumbnails all work.
- Not yet tested on real hardware: Windows Recycle Bin **restore**, real SMB shares, a real Mac.
- The default-file-manager feature is **labelled experimental in Settings** (header + a note under
  the toggle) because of the Win+E failure below. Jakub's call on 2026-10-28.
- **Win+E takeover does not work on Windows 11 build 26300** and is unfixed. The registry is
  provably correct (`Directory\shell` = `skjfiles` with a valid `command` subkey; the CLSID
  `opennewwindow\command` written and `DelegateExecute` present, confirmed through the merged
  HKCR view), `SHChangeNotify(SHCNE_ASSOCCHANGED)` is now called, and **restarting Explorer did
  not help** — so it is not a caching problem. Prime suspect: `{52205fd8-…}` is the Windows 10
  Quick-access target and Win11 has moved Win+E elsewhere; `{f874310e-b6b7-47dc-bc84-b9e6b38f5903}`
  exists on 26300 but with an empty default value. **Already tried and ruled out on 2026-10-28:**
  `SHChangeNotify`, restarting Explorer, and writing `opennewwindow\command` by hand under both
  `{f874310e-b6b7-47dc-bc84-b9e6b38f5903}` and `{679f85cb-0220-4080-b29b-5540cc05aab6}` — Win+E
  still opened Explorer every time. Don't repeat those; look for how 26300 actually dispatches Win+E.
  Note that double-clicking a folder **inside an Explorer window** navigates and never invokes the
  shell verb — that is Windows behaviour, not a bug, so test folder association from the desktop.

## Roadmap

- **0.5–0.9 (next):** installers (Windows: Start menu, uninstaller, "Open folder with SKJ Files";
  Fedora RPM / Flatpak / AppImage; macOS .dmg), GitHub Actions builds for all 3 OSes in the
  SKJ-Technology org (ask first!), update check, SignPath signing (needs GPL-3.0 + public repo — ask first),
  testing on real machines (Windows laptop, Fedora KDE Wayland, a Mac), stress tests (network drives,
  100k+ files, smaller binary), crash log file, settings robustness.
- **1.0:** fix bugs from Jakub's daily use; release page on skj-tech.online + GitHub release.
- 7z / RAR stay in SKJ Archiver.
