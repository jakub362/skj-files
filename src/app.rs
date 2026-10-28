//! SKJ Files — application state and logic. Lives on the UI thread; background work reports back
//! through `slint::invoke_from_event_loop` + `with_app`.

use crate::fsx::{self, Conflict, Drive, Entry, OpKind};
use crate::thumbs;
use crate::i18n::{self, ts};
use crate::tf;
use crate::settings::Settings;
use crate::{AppWindow, ColData, ColRow, Crumb, FileRow, MenuEntry, PropRow, SideItem, TabInfo, Theme, S};
use slint::{Color, ComponentHandle, DataTransfer, Model, ModelRc, SharedPixelBuffer, Rgba8Pixel, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub const ACCENTS: [(u8, u8, u8); 7] = [
    (0x6C, 0x8C, 0xFF),
    (0x8B, 0x5C, 0xF6),
    (0x14, 0xB8, 0xA6),
    (0x22, 0xC5, 0x5E),
    (0xF5, 0x9E, 0x0B),
    (0xEF, 0x44, 0x44),
    (0xEC, 0x48, 0x99),
];

pub const TAG_COLORS: [(u8, u8, u8); 6] = [(0xEF, 0x44, 0x44), (0xF9, 0x73, 0x16), (0xEA, 0xB3, 0x08), (0x22, 0xC5, 0x5E), (0x3B, 0x82, 0xF6), (0xA8, 0x55, 0xF7)];
pub const TAG_NAMES: [&str; 6] = ["Red", "Orange", "Yellow", "Green", "Blue", "Purple"];

/// Toolbar buttons: (id, label, shown by default)
pub const TOOLBAR: &[(&str, &str, bool)] = &[
    ("cut", "Cut", true),
    ("copy", "Copy", true),
    ("paste", "Paste", true),
    ("rename", "Rename", true),
    ("delete", "Delete", true),
    ("undo", "Undo", true),
    ("sort", "Sort", true),
    ("tools", "Tools", true),
    ("view", "View", true),
    ("hidden", "Hidden files", true),
    ("dual", "Dual pane", true),
    ("preview", "Preview pane", true),
    ("palette", "Command palette", false),
    ("newtab", "New tab", false),
    ("terminal", "Open in terminal", false),
    ("vscode", "Open in VS Code", false),
    ("hashes", "Hashes", false),
    ("compress", "Compress to ZIP", false),
    ("properties", "Properties", false),
    ("bin", "Recycle Bin", false),
];

/// Built-in shortcuts (shown in the palette and the shortcut settings).
const DEFAULT_KEYS: &[(&str, &str)] = &[
    ("palette", "Ctrl+Shift+P"),
    ("new-folder", "Ctrl+Shift+N"),
    ("new-tab", "Ctrl+T"),
    ("close-tab", "Ctrl+W"),
    ("reopen-tab", "Ctrl+Shift+T"),
    ("new-window", "Ctrl+N"),
    ("copy", "Ctrl+C"),
    ("cut", "Ctrl+X"),
    ("paste", "Ctrl+V"),
    ("copy-path", "Ctrl+Shift+C"),
    ("rename", "F2"),
    ("trash", "Del"),
    ("delete-perm", "Shift+Del"),
    ("undo", "Ctrl+Z"),
    ("select-all", "Ctrl+A"),
    ("refresh", "F5"),
    ("properties", "Alt+Enter"),
    ("hashes", "Ctrl+Shift+H"),
    ("quicklook", "Space"),
    ("toggle-hidden", "Ctrl+H"),
    ("toggle-preview", "Ctrl+P"),
    ("settings", "Ctrl+,"),
    ("edit-path", "Ctrl+L"),
    ("focus-search", "Ctrl+F"),
    ("go-back", "Alt+Left"),
    ("go-forward", "Alt+Right"),
    ("go-up", "Alt+Up"),
];

/// Commands whose shortcut can be changed in Settings.
const REBINDABLE: &[(&str, &str)] = &[
    ("palette", "Command palette"),
    ("new-folder", "New folder"),
    ("new-file", "New file"),
    ("new-tab", "New tab"),
    ("close-tab", "Close tab"),
    ("reopen-tab", "Reopen closed tab"),
    ("new-window", "New window"),
    ("rename", "Rename"),
    ("copy-path", "Copy path"),
    ("hashes", "Hashes"),
    ("compress", "Compress to ZIP"),
    ("quicklook", "Quick look"),
    ("terminal", "Open in terminal"),
    ("vscode", "Open in VS Code"),
    ("toggle-hidden", "Hidden files"),
    ("toggle-preview", "Preview pane"),
    ("toggle-dual", "Dual pane"),
    ("view:0", "Details view"),
    ("view:1", "Grid view"),
    ("view:2", "Column view"),
    ("properties", "Properties"),
    ("refresh", "Refresh"),
    ("go-home", "Go to Home"),
    ("open-bin", "Go to the Recycle Bin"),
    ("connect", "Connect to server"),
    ("settings", "Settings"),
    ("shell-menu", "Show more options (Windows menu)"),
];

#[derive(Clone)]
struct PalItem {
    label: String,
    /// English label, so English words keep working in the Polish UI
    en: String,
    icon: i32,
    action: String,
    hint: String,
}

fn tag_color(i: usize) -> Color {
    let (r, g, b) = TAG_COLORS[i];
    Color::from_rgb_u8(r, g, b)
}

#[derive(Clone, Debug)]
enum UndoAct {
    Renamed(Vec<(PathBuf, PathBuf)>),
    Created(Vec<PathBuf>),
    Moved(Vec<(PathBuf, PathBuf)>),
    MovedThenCreated(Vec<(PathBuf, PathBuf)>, PathBuf),
    Trashed(Vec<PathBuf>),
}

#[derive(Clone, Debug, PartialEq)]
enum UndoHint {
    None,
    Normal,
    FolderFromSel(PathBuf),
}

thread_local! {
    pub static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Run `f` with the app state. Re-entrant calls are deferred to the next event-loop turn.
pub fn with_app(f: impl FnOnce(&mut App) + 'static) {
    let pending: Option<Box<dyn FnOnce(&mut App)>> = APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut g) => {
            if let Some(a) = g.as_mut() {
                f(a);
            }
            None
        }
        Err(_) => Some(Box::new(f) as Box<dyn FnOnce(&mut App)>),
    });
    if let Some(f) = pending {
        slint::Timer::single_shot(Duration::ZERO, move || with_app(f));
    }
}

pub fn with_app_ret<R: Default>(f: impl FnOnce(&mut App) -> R) -> R {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut g) => g.as_mut().map(f).unwrap_or_default(),
        Err(_) => R::default(),
    })
}

/// Switch Rust strings and the Slint UI (@tr) to `code` ("en" / "pl").
pub fn apply_language(code: &str) {
    i18n::set(code);
    let _ = slint::select_bundled_translation(if code == "pl" { "pl" } else { "" });
}

fn ss(s: impl AsRef<str>) -> SharedString {
    SharedString::from(s.as_ref())
}

// ───────────────────────── types ─────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum Loc {
    Home,
    Dir(PathBuf),
    Search { root: PathBuf, query: String },
    Tag(u8),
    /// the Recycle Bin / Trash
    Bin,
}

impl Loc {
    fn dir(&self) -> Option<&Path> {
        match self {
            Loc::Dir(p) => Some(p),
            _ => None,
        }
    }
}

type ListSlot = Arc<(Mutex<Option<Result<Vec<Entry>, String>>>, Condvar)>;

pub struct Pane {
    pub loc: Loc,
    back: Vec<Loc>,
    fwd: Vec<Loc>,
    entries: Vec<Entry>,
    view: Vec<usize>,
    selected: HashSet<PathBuf>,
    anchor: Option<usize>,
    cursor: Option<usize>,
    filter: String,
    loading: bool,
    gen: u64,
    error: Option<String>,
    model: Rc<VecModel<FileRow>>,
    pending_select: Vec<PathBuf>,
    searched: usize,
    search_done: bool,
}

impl Pane {
    fn new(loc: Loc) -> Pane {
        Pane {
            loc,
            back: Vec::new(),
            fwd: Vec::new(),
            entries: Vec::new(),
            view: Vec::new(),
            selected: HashSet::new(),
            anchor: None,
            cursor: None,
            filter: String::new(),
            loading: false,
            gen: 0,
            error: None,
            model: Rc::new(VecModel::default()),
            pending_select: Vec::new(),
            searched: 0,
            search_done: true,
        }
    }

    fn entry(&self, view_idx: usize) -> Option<&Entry> {
        self.view.get(view_idx).and_then(|&i| self.entries.get(i))
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        // keep on-screen order
        self.view.iter().filter_map(|&i| self.entries.get(i)).filter(|e| self.selected.contains(&e.path)).map(|e| e.path.clone()).collect()
    }

    fn selected_entries(&self) -> Vec<&Entry> {
        self.view.iter().filter_map(|&i| self.entries.get(i)).filter(|e| self.selected.contains(&e.path)).collect()
    }
}

pub struct Tab {
    id: u64,
    panes: [Pane; 2],
    dual: bool,
    active: usize,
}

#[derive(Clone)]
enum Dialog {
    None,
    FolderFromSel { dir: PathBuf, items: Vec<PathBuf> },
    ConfirmTrash { items: Vec<PathBuf> },
    ConfirmDelete { items: Vec<PathBuf> },
    ConfirmPurge { items: Vec<PathBuf> },
    ConfirmEmpty,
    Connect,
    Settings,
    Props { items: Vec<PathBuf> },
    Hashes,
    Batch,
    Conflict,
}

/// A copy/move/… waiting to run (or to have its name clashes decided).
struct PendingOp {
    kind: OpKind,
    srcs: Vec<PathBuf>,
    dest: Option<PathBuf>,
    hint: UndoHint,
    policies: Option<HashMap<PathBuf, Conflict>>,
}

struct ConflictState {
    op: PendingOp,
    list: Vec<(PathBuf, PathBuf)>,
    idx: usize,
    decided: HashMap<PathBuf, Conflict>,
}

#[derive(Clone, Default)]
struct MenuCtx {
    pane: usize,
    targets: Vec<PathBuf>,
    side_path: Option<PathBuf>,
}

pub struct App {
    ui: AppWindow,
    tabs: Vec<Tab>,
    cur: usize,
    next_id: u64,
    pub s: Settings,
    clip: Option<(Vec<PathBuf>, bool)>,
    #[cfg(windows)]
    clip_seq: u32,
    drives: Vec<Drive>,
    quick: Vec<(String, i32, PathBuf)>,
    op_running: bool,
    op_queue: VecDeque<PendingOp>,
    op_cancel: Arc<AtomicBool>,
    conflict: Option<ConflictState>,
    renaming: Option<(usize, PathBuf)>,
    rename_after_list: Option<PathBuf>,
    refresh_after_rename: Vec<PathBuf>,
    marquee_base: HashSet<PathBuf>,
    bin_items: HashMap<PathBuf, fsx::BinItem>,
    thumb_pool: thumbs::Pool,
    pal_all: Vec<PalItem>,
    pal_view: Vec<usize>,
    closed_tabs: Vec<Loc>,
    menu_pos: (f32, f32),
    menu_tab: usize,
    col_gen: [u64; 2],
    col_paths: [Vec<PathBuf>; 2],
    col_rows: [Vec<Vec<String>>; 2],
    child_dir: [Option<PathBuf>; 2],
    child_rows: [Vec<String>; 2],
    select_first: [bool; 2],
    capture_action: Option<String>,
    tabs_model: Rc<VecModel<TabInfo>>,
    tab_drags_model: Rc<VecModel<DataTransfer>>,
    #[cfg(not(windows))]
    ext_drop: Vec<PathBuf>,
    watcher: Option<notify::RecommendedWatcher>,
    watched: HashSet<PathBuf>,
    type_buf: String,
    type_at: Instant,
    pv_gen: Arc<AtomicU64>,
    search_gen: Arc<AtomicU64>,
    props_gen: Arc<AtomicU64>,
    toast_gen: u64,
    dialog: Dialog,
    menu: MenuCtx,
    crumb_paths: Vec<PathBuf>,
    sys_clip: Option<arboard::Clipboard>,
    list_gen: u64,
    ensure_gen: i32,
    props_rows: Vec<(String, String)>,
    thumbs: HashMap<PathBuf, (u32, Option<std::time::SystemTime>, slint::Image)>,
    thumb_gen: Arc<AtomicU64>,
    tags: HashMap<PathBuf, u8>,
    undo: Vec<UndoAct>,
    ql_gen: Arc<AtomicU64>,
    hash_gen: Arc<AtomicU64>,
    hash_rows: Vec<(String, String)>,
    hash_multi: bool,
    batch_items: Vec<PathBuf>,
    pending_collapse: Option<(usize, usize)>,
    starting: bool,
}

impl App {
    pub fn new(ui: AppWindow, s: Settings) -> App {
        App {
            ui,
            tabs: Vec::new(),
            cur: 0,
            next_id: 1,
            s,
            clip: None,
            #[cfg(windows)]
            clip_seq: 0,
            drives: Vec::new(),
            quick: Vec::new(),
            op_running: false,
            op_queue: VecDeque::new(),
            op_cancel: Arc::new(AtomicBool::new(false)),
            conflict: None,
            renaming: None,
            rename_after_list: None,
            refresh_after_rename: Vec::new(),
            marquee_base: HashSet::new(),
            bin_items: HashMap::new(),
            pal_all: Vec::new(),
            pal_view: Vec::new(),
            closed_tabs: Vec::new(),
            menu_pos: (0.0, 0.0),
            menu_tab: 0,
            col_gen: [0, 0],
            col_paths: [Vec::new(), Vec::new()],
            col_rows: [Vec::new(), Vec::new()],
            child_dir: [None, None],
            child_rows: [Vec::new(), Vec::new()],
            select_first: [false, false],
            capture_action: None,
            tabs_model: Rc::new(VecModel::default()),
            tab_drags_model: Rc::new(VecModel::default()),
            thumb_pool: thumbs::Pool::new(3, |gen, path, size, t| {
                let _ = slint::invoke_from_event_loop(move || with_app(move |a| a.on_thumb(gen, path, size, t)));
            }),
            #[cfg(not(windows))]
            ext_drop: Vec::new(),
            watcher: None,
            watched: HashSet::new(),
            type_buf: String::new(),
            type_at: Instant::now(),
            pv_gen: Arc::new(AtomicU64::new(0)),
            search_gen: Arc::new(AtomicU64::new(0)),
            props_gen: Arc::new(AtomicU64::new(0)),
            toast_gen: 0,
            dialog: Dialog::None,
            menu: MenuCtx::default(),
            crumb_paths: Vec::new(),
            sys_clip: None,
            list_gen: 0,
            ensure_gen: 0,
            props_rows: Vec::new(),
            thumbs: HashMap::new(),
            thumb_gen: Arc::new(AtomicU64::new(0)),
            tags: HashMap::new(),
            undo: Vec::new(),
            ql_gen: Arc::new(AtomicU64::new(0)),
            hash_gen: Arc::new(AtomicU64::new(0)),
            hash_rows: Vec::new(),
            hash_multi: false,
            batch_items: Vec::new(),
            pending_collapse: None,
            starting: true,
        }
    }

    fn g(&self) -> S<'_> {
        self.ui.global::<S>()
    }

    // ───────────── startup ─────────────

    pub fn start(&mut self, initial: Vec<PathBuf>) {
        self.apply_theme();
        let g = self.g();
        g.set_tabs(ModelRc::from(self.tabs_model.clone()));
        g.set_tab_drags(ModelRc::from(self.tab_drags_model.clone()));
        g.set_mono_font(ss(if cfg!(windows) { "Consolas" } else if cfg!(target_os = "macos") { "Menlo" } else { "monospace" }));
        g.set_version(ss(env!("CARGO_PKG_VERSION")));
        g.set_lang_setting(ss(&self.s.lang));
        self.tags = self.s.tags.iter().cloned().collect();
        self.sync_settings_ui();
        self.load_quick();
        slint::Timer::single_shot(Duration::from_millis(1), || {
            with_app(|a| {
                a.apply_titlebar();
                #[cfg(not(windows))]
                a.hook_external_drops();
            })
        });
        self.start_watcher();

        // tabs: folders passed on the command line, else the last session, else Home
        let mut opened = false;
        for p in initial {
            if p.is_dir() {
                self.open_tab(Loc::Dir(p), false);
                opened = true;
            } else if let Some(parent) = p.parent().filter(|_| p.is_file()) {
                self.open_tab_select(Loc::Dir(parent.to_path_buf()), vec![p.clone()], false);
                opened = true;
            }
        }
        if opened {
            self.s.active_tab = self.tabs.len() - 1;
        }
        if !opened && self.s.restore {
            let saved = self.s.tabs.clone();
            for t in saved {
                let loc = if t == "::home" { Loc::Home } else { Loc::Dir(PathBuf::from(&t)) };
                if let Loc::Dir(p) = &loc {
                    if !p.is_dir() {
                        continue;
                    }
                }
                self.open_tab(loc, false);
                opened = true;
            }
        }
        if !opened {
            self.open_tab(Loc::Home, false);
        }
        self.cur = self.s.active_tab.min(self.tabs.len() - 1);
        self.sync_all();
        self.start_drive_poll();
        self.refresh_recent_files();
        self.sync_shortcut_rows();
        self.starting = false;
    }

    fn apply_theme(&self) {
        let t = self.ui.global::<Theme>();
        t.set_dark(self.s.dark);
        let (r, g, b) = ACCENTS[self.s.accent.min(ACCENTS.len() - 1)];
        t.set_accent(Color::from_rgb_u8(r, g, b));
    }

    fn sync_settings_ui(&self) {
        let g = self.g();
        g.set_show_hidden(self.s.show_hidden);
        g.set_set_show_ext(self.s.show_ext);
        g.set_set_restore(self.s.restore);
        g.set_set_confirm_delete(self.s.confirm_delete);
        g.set_accent_index(self.s.accent as i32);
        g.set_view_mode(self.s.view_mode);
        g.set_sort_key(self.s.sort_key);
        g.set_sort_asc(self.s.sort_asc);
        g.set_show_preview(self.s.show_preview);
        g.set_friendly_dates(self.s.friendly_dates);
        g.set_grid_size(self.s.grid_size);
        self.sync_columns();
        self.sync_toolbar();
        g.set_fm_supported(crate::sysint::supported());
        g.set_on_windows(cfg!(windows));
        g.set_is_default_fm(crate::sysint::is_default());
        g.set_exe_path(ss(std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()));
    }

    fn toolbar_on(&self, id: &str) -> bool {
        if self.s.toolbar_set {
            self.s.toolbar.iter().any(|x| x == id)
        } else {
            TOOLBAR.iter().any(|(i, _, d)| *i == id && *d)
        }
    }

    fn sync_toolbar(&self) {
        let g = self.g();
        let flags: Vec<bool> = TOOLBAR.iter().map(|(id, _, _)| self.toolbar_on(id)).collect();
        g.set_tb(ModelRc::new(VecModel::from(flags)));
        let rows: Vec<MenuEntry> = TOOLBAR.iter().map(|(id, label, _)| chk(label, &format!("tb:{}", id), self.toolbar_on(id))).collect();
        g.set_tb_rows(ModelRc::new(VecModel::from(rows)));
    }

    fn toggle_toolbar(&mut self, id: &str) {
        let mut on: Vec<String> = TOOLBAR.iter().filter(|(i, _, _)| self.toolbar_on(i)).map(|(i, _, _)| i.to_string()).collect();
        if let Some(pos) = on.iter().position(|x| x == id) {
            on.remove(pos);
        } else {
            on.push(id.to_string());
        }
        self.s.toolbar = on;
        self.s.toolbar_set = true;
        self.s.save();
        self.sync_toolbar();
    }

    fn sync_columns(&self) {
        let g = self.g();
        let w = self.s.col_w;
        g.set_col_w_date(w[0]);
        g.set_col_w_created(w[1]);
        g.set_col_w_type(w[2]);
        g.set_col_w_size(w[3]);
        g.set_col_w_loc(w[4]);
        let c = self.s.col_show;
        g.set_col_date(c[0]);
        g.set_col_created(c[1]);
        g.set_col_type(c[2]);
        g.set_col_size(c[3]);
    }

    /// Files dragged in from other apps on macOS / Linux (X11). winit reports them without a
    /// position, so they go into the active pane's folder. (Windows uses real OLE drag and drop
    /// with per-folder targets, see the bundled winit backend.)
    #[cfg(not(windows))]
    fn hook_external_drops(&self) {
        use slint::winit_030::{winit::event::WindowEvent as WE, EventResult, WinitWindowAccessor};
        self.ui.window().on_winit_window_event(|_w, ev| {
            match ev {
                WE::HoveredFile(_) => with_app(|a| a.g().set_ext_drop(true)),
                WE::HoveredFileCancelled => with_app(|a| a.g().set_ext_drop(false)),
                WE::DroppedFile(p) => {
                    let p = p.clone();
                    with_app(move |a| a.ext_dropped(p));
                }
                _ => {}
            }
            EventResult::Propagate
        });
    }

    #[cfg(not(windows))]
    fn ext_dropped(&mut self, p: PathBuf) {
        let first = self.ext_drop.is_empty();
        self.ext_drop.push(p);
        if first {
            // winit sends one event per file; take them all together
            slint::Timer::single_shot(Duration::from_millis(60), || with_app(|a| a.ext_drop_flush()));
        }
    }

    #[cfg(not(windows))]
    fn ext_drop_flush(&mut self) {
        self.g().set_ext_drop(false);
        let paths = std::mem::take(&mut self.ext_drop);
        if paths.is_empty() {
            return;
        }
        let Some(dest) = self.current_dir() else {
            self.toast(ts("Open a folder first, then drop the files on it"));
            return;
        };
        if paths.iter().all(|p| p.parent() == Some(dest.as_path())) {
            return;
        }
        let other_drive = paths.iter().any(|p| self.drive_for(p).map(|d| d.mount.clone()) != self.drive_for(&dest).map(|d| d.mount.clone()));
        self.start_op(if other_drive { OpKind::Copy } else { OpKind::Move }, paths, Some(dest));
    }

    fn load_quick(&mut self) {
        self.quick.clear();
        if self.s.pins_set {
            for p in self.s.pins.clone() {
                let label = fsx::file_name(&p);
                let kind = fsx::kind_for_place(&p);
                self.quick.push((label, kind, p));
            }
        } else {
            for p in fsx::default_quick_access() {
                self.quick.push((p.label, p.kind, p.path));
            }
        }
        self.sync_sidebar();
    }

    fn start_drive_poll(&mut self) {
        std::thread::spawn(move || {
            let mut last: Vec<Drive> = Vec::new();
            loop {
                let d = fsx::list_drives();
                // only push changes (ignore tiny free-space jitter)
                let changed = d.len() != last.len()
                    || d.iter().zip(last.iter()).any(|(a, b)| a.mount != b.mount || a.label != b.label || a.free / (64 << 20) != b.free / (64 << 20));
                if changed {
                    last = d.clone();
                    let _ = slint::invoke_from_event_loop(move || with_app(move |a| a.on_drives(d)));
                }
                std::thread::sleep(Duration::from_secs(3));
            }
        });
    }

    fn on_drives(&mut self, d: Vec<Drive>) {
        self.drives = d;
        self.sync_sidebar();
        self.sync_status();
        self.refresh_titles();
    }

    // ───────────── watcher ─────────────

    fn start_watcher(&mut self) {
        let pending: Arc<Mutex<HashSet<PathBuf>>> = Arc::new(Mutex::new(HashSet::new()));
        let scheduled = Arc::new(AtomicBool::new(false));
        let w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(ev) = res else { return };
            if matches!(ev.kind, notify::EventKind::Access(_)) {
                return;
            }
            {
                let mut p = pending.lock().unwrap();
                for path in &ev.paths {
                    if let Some(parent) = path.parent() {
                        p.insert(parent.to_path_buf());
                    }
                    p.insert(path.clone());
                }
            }
            if !scheduled.swap(true, Ordering::SeqCst) {
                let pending = pending.clone();
                let scheduled = scheduled.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(350));
                    scheduled.store(false, Ordering::SeqCst);
                    let dirs: Vec<PathBuf> = pending.lock().unwrap().drain().collect();
                    let _ = slint::invoke_from_event_loop(move || with_app(move |a| a.on_fs_changed(dirs)));
                });
            }
        });
        self.watcher = w.ok();
    }

    fn update_watches(&mut self) {
        use notify::Watcher;
        let mut want: HashSet<PathBuf> = HashSet::new();
        for t in &self.tabs {
            for (i, p) in t.panes.iter().enumerate() {
                if i == 1 && !t.dual {
                    continue;
                }
                if let Loc::Dir(d) = &p.loc {
                    want.insert(d.clone());
                }
            }
        }
        let Some(w) = self.watcher.as_mut() else { return };
        for old in self.watched.difference(&want).cloned().collect::<Vec<_>>() {
            let _ = w.unwatch(&old);
            self.watched.remove(&old);
        }
        for new in want.difference(&self.watched.clone()).cloned().collect::<Vec<_>>() {
            if w.watch(&new, notify::RecursiveMode::NonRecursive).is_ok() {
                self.watched.insert(new);
            }
        }
    }

    fn on_fs_changed(&mut self, dirs: Vec<PathBuf>) {
        let set: HashSet<PathBuf> = dirs.into_iter().collect();
        let mut todo = Vec::new();
        for (ti, t) in self.tabs.iter().enumerate() {
            for pi in 0..2 {
                if pi == 1 && !t.dual {
                    continue;
                }
                if let Loc::Dir(d) = &t.panes[pi].loc {
                    if set.contains(d) {
                        todo.push((ti, pi));
                    }
                }
            }
        }
        for (ti, pi) in todo {
            // a refresh would rebuild the row being renamed and drop what was typed
            if ti == self.cur && self.renaming.as_ref().map(|(rp, _)| *rp == pi).unwrap_or(false) {
                if let Some(d) = self.tabs[ti].panes[pi].loc.dir() {
                    self.refresh_after_rename.push(d.to_path_buf());
                }
                continue;
            }
            self.start_listing(ti, pi, false);
        }
    }

    // ───────────── tabs ─────────────

    pub fn open_tab(&mut self, loc: Loc, activate: bool) {
        self.open_tab_select(loc, Vec::new(), activate);
    }

    pub fn open_tab_select(&mut self, loc: Loc, select: Vec<PathBuf>, activate: bool) {
        let id = self.next_id;
        self.next_id += 1;
        let mut first = Pane::new(loc.clone());
        first.pending_select = select;
        let tab = Tab { id, panes: [first, Pane::new(Loc::Home)], dual: false, active: 0 };
        self.tabs.push(tab);
        let ti = self.tabs.len() - 1;
        if let Loc::Dir(_) | Loc::Search { .. } | Loc::Bin = loc {
            self.start_listing(ti, 0, true);
        }
        if activate {
            self.cur = ti;
            self.sync_all();
        } else {
            self.refresh_titles();
        }
        self.update_watches();
    }

    fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        self.end_rename();
        if self.tabs.len() == 1 {
            // closing the last tab resets it to Home, like a browser keeping the window open
            self.ui.hide().ok();
            return;
        }
        let closed = self.tabs[i].panes[self.tabs[i].active].loc.clone();
        self.closed_tabs.push(closed);
        if self.closed_tabs.len() > 20 {
            self.closed_tabs.remove(0);
        }
        self.tabs.remove(i);
        if self.cur >= self.tabs.len() {
            self.cur = self.tabs.len() - 1;
        } else if i < self.cur {
            self.cur -= 1;
        }
        self.update_watches();
        self.sync_all();
    }

    fn select_tab(&mut self, i: usize) {
        if i < self.tabs.len() && i != self.cur {
            self.end_rename();
            self.cur = i;
            self.sync_all();
            self.request_thumbs();
        }
    }

    // ───────────── navigation ─────────────

    fn ap(&self) -> usize {
        self.tabs[self.cur].active
    }

    fn pane(&self) -> &Pane {
        let t = &self.tabs[self.cur];
        &t.panes[t.active]
    }

    fn pane_mut(&mut self) -> &mut Pane {
        let t = &mut self.tabs[self.cur];
        &mut t.panes[t.active]
    }

    pub fn navigate(&mut self, pi: usize, loc: Loc, push_history: bool) {
        self.end_rename();
        let ti = self.cur;
        {
            let p = &mut self.tabs[ti].panes[pi];
            if p.loc == loc && !matches!(loc, Loc::Search { .. }) {
                return;
            }
            if push_history {
                let old = p.loc.clone();
                p.back.push(old);
                if p.back.len() > 100 {
                    p.back.remove(0);
                }
                p.fwd.clear();
            }
            p.loc = loc.clone();
            p.filter.clear();
            p.selected.clear();
            p.anchor = None;
            p.cursor = None;
            p.entries.clear();
            p.view.clear();
            p.model.set_vec(Vec::new());
            p.error = None;
        }
        self.tabs[ti].active = pi;
        if let Loc::Dir(d) = &loc {
            self.add_recent(d.clone());
        }
        self.start_listing(ti, pi, true);
        self.ensure_visible(pi, -1);
        self.update_watches();
        self.sync_all();
    }

    fn go_back(&mut self) {
        let pi = self.ap();
        let p = self.pane_mut();
        if let Some(prev) = p.back.pop() {
            let cur = p.loc.clone();
            p.fwd.push(cur.clone());
            if let (Loc::Dir(c), Loc::Dir(_)) = (&cur, &prev) {
                p.pending_select = vec![c.clone()];
            }
            self.navigate_raw(pi, prev);
        }
    }

    fn go_forward(&mut self) {
        let pi = self.ap();
        let p = self.pane_mut();
        if let Some(next) = p.fwd.pop() {
            let cur = p.loc.clone();
            p.back.push(cur);
            self.navigate_raw(pi, next);
        }
    }

    fn go_up(&mut self) {
        let pi = self.ap();
        let loc = self.pane().loc.clone();
        match loc {
            Loc::Dir(d) => {
                if let Some(parent) = d.parent() {
                    let parent = parent.to_path_buf();
                    self.navigate_select(pi, parent, d.clone());
                } else {
                    self.navigate(pi, Loc::Home, true);
                }
            }
            Loc::Search { root, .. } => self.navigate(pi, Loc::Dir(root), true),
            Loc::Tag(_) | Loc::Bin => self.navigate(pi, Loc::Home, true),
            Loc::Home => {}
        }
    }

    /// Navigate without touching history (used by back/forward).
    fn navigate_raw(&mut self, pi: usize, loc: Loc) {
        let keep = std::mem::take(&mut self.pane_mut().pending_select);
        let ti = self.cur;
        {
            let p = &mut self.tabs[ti].panes[pi];
            p.loc = loc;
            p.filter.clear();
            p.selected.clear();
            p.anchor = None;
            p.cursor = None;
            p.entries.clear();
            p.view.clear();
            p.model.set_vec(Vec::new());
            p.pending_select = keep;
        }
        self.start_listing(ti, pi, true);
        self.ensure_visible(pi, -1);
        self.update_watches();
        self.sync_all();
    }

    fn open_path_str(&mut self, s: &str, new_tab: bool) {
        if s == "::home" {
            if new_tab { self.open_tab(Loc::Home, true) } else { self.navigate(self.ap(), Loc::Home, true) }
            return;
        }
        if let Some(i) = s.strip_prefix("::tag:") {
            let loc = Loc::Tag(i.parse().unwrap_or(0));
            if new_tab { self.open_tab(loc, true) } else { self.navigate(self.ap(), loc, true) }
            return;
        }
        if s == "::connect" {
            self.open_connect();
            return;
        }
        if let Some(i) = s.strip_prefix("::server:") {
            if let Some(srv) = self.s.servers.get(i.parse::<usize>().unwrap_or(usize::MAX)).cloned() {
                self.open_server(srv, String::new(), String::new(), new_tab);
            }
            return;
        }
        if let Some(f) = s.strip_prefix("::recentfile:") {
            let p = PathBuf::from(f);
            match fsx::open_file(&p) {
                Ok(_) => self.remember_file(p),
                Err(e) => self.toast(tf!("Couldn't open {}: {}", fsx::file_name(&p), e)),
            }
            return;
        }
        if s == "::bin" || s == "::recycle" {
            if new_tab { self.open_tab(Loc::Bin, true) } else { self.navigate(self.ap(), Loc::Bin, true) }
            return;
        }
        let p = PathBuf::from(s);
        if new_tab {
            self.open_tab(Loc::Dir(p), true);
        } else {
            self.navigate(self.ap(), Loc::Dir(p), true);
        }
    }

    fn path_entered(&mut self, text: &str) {
        self.g().set_edit_path(false);
        let p = fsx::expand_user(text);
        if p.as_os_str().is_empty() {
            return;
        }
        if p.is_dir() {
            self.navigate(self.ap(), Loc::Dir(p), true);
        } else if p.is_file() {
            if let Some(parent) = p.parent() {
                let parent = parent.to_path_buf();
                let pi = self.ap();
                self.navigate_select(pi, parent, p.clone());
            }
        } else {
            self.toast(tf!("Can't find \"{}\"", text.trim()));
        }
        self.ui.invoke_focus_main();
    }

    fn add_recent(&mut self, d: PathBuf) {
        self.s.recent.retain(|x| x != &d);
        self.s.recent.insert(0, d);
        self.s.recent.truncate(12);
    }

    // ───────────── listing ─────────────

    fn start_listing(&mut self, ti: usize, pi: usize, fresh: bool) {
        let tab_id = self.tabs[ti].id;
        self.list_gen += 1;
        let gen = self.list_gen;
        let loc = self.tabs[ti].panes[pi].loc.clone();
        {
            let p = &mut self.tabs[ti].panes[pi];
            p.gen = gen;
            if fresh {
                p.loading = true;
            }
        }
        match loc {
            Loc::Home => {
                self.tabs[ti].panes[pi].loading = false;
            }
            Loc::Bin => {
                std::thread::spawn(move || {
                    let res = fsx::bin_list();
                    let _ = slint::invoke_from_event_loop(move || with_app(move |a| a.on_bin_listed(tab_id, pi, gen, res)));
                });
            }
            Loc::Tag(i) => {
                let bit = 1u8 << i;
                let mut v = Vec::new();
                for (path, m) in &self.tags {
                    if m & bit == 0 {
                        continue;
                    }
                    if let Ok(md) = std::fs::metadata(path) {
                        let loc = path.parent().map(|x| x.display().to_string()).unwrap_or_default();
                        v.push(fsx::entry_from(path.clone(), fsx::file_name(path), &md, loc));
                    }
                }
                self.on_listed(tab_id, pi, gen, Ok(v));
            }
            Loc::Dir(path) => {
                let slot: ListSlot = Arc::new((Mutex::new(None), Condvar::new()));
                let s2 = slot.clone();
                std::thread::spawn(move || {
                    let res = fsx::list_dir(&path);
                    {
                        let (m, cv) = &*s2;
                        *m.lock().unwrap() = Some(res);
                        cv.notify_all();
                    }
                    let s3 = s2.clone();
                    let _ = slint::invoke_from_event_loop(move || with_app(move |a| a.take_listing(tab_id, pi, gen, s3)));
                });
                // Most folders list in a few ms: wait briefly so the view never flashes "Loading…".
                // (Not during startup: the window must appear first.)
                if self.starting {
                    return;
                }
                let (m, cv) = &*slot;
                let guard = m.lock().unwrap();
                let (mut guard, _) = cv.wait_timeout_while(guard, Duration::from_millis(45), |r| r.is_none()).unwrap();
                if let Some(res) = guard.take() {
                    drop(guard);
                    self.on_listed(tab_id, pi, gen, res);
                }
            }
            Loc::Search { root, query } => {
                let g = self.search_gen.fetch_add(1, Ordering::SeqCst) + 1;
                let cancel = self.search_gen.clone();
                let hidden = self.s.show_hidden;
                {
                    let p = &mut self.tabs[ti].panes[pi];
                    p.entries.clear();
                    p.searched = 0;
                    p.search_done = false;
                    p.loading = true;
                }
                std::thread::spawn(move || {
                    let c2 = cancel.clone();
                    fsx::search(&root, &query, hidden, &move || c2.load(Ordering::SeqCst) != g, &mut |batch, scanned| {
                        let _ = slint::invoke_from_event_loop(move || with_app(move |a| a.on_search_batch(tab_id, pi, gen, batch, scanned, false)));
                    });
                    if cancel.load(Ordering::SeqCst) == g {
                        let _ = slint::invoke_from_event_loop(move || with_app(move |a| a.on_search_batch(tab_id, pi, gen, Vec::new(), 0, true)));
                    }
                });
            }
        }
    }

    fn on_bin_listed(&mut self, tab_id: u64, pi: usize, gen: u64, res: Result<Vec<(Entry, fsx::BinItem)>, String>) {
        let res = res.map(|v| {
            let mut out = Vec::with_capacity(v.len());
            for (e, it) in v {
                self.bin_items.insert(e.path.clone(), it);
                out.push(e);
            }
            out
        });
        self.on_listed(tab_id, pi, gen, res);
    }

    fn refresh_bin_views(&mut self) {
        for ti in 0..self.tabs.len() {
            for pi in 0..2 {
                if self.tabs[ti].panes[pi].loc == Loc::Bin {
                    self.start_listing(ti, pi, false);
                }
            }
        }
    }

    fn take_listing(&mut self, tab_id: u64, pi: usize, gen: u64, slot: ListSlot) {
        let res = slot.0.lock().unwrap().take();
        if let Some(res) = res {
            self.on_listed(tab_id, pi, gen, res);
        }
    }

    fn tab_index(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    fn on_listed(&mut self, tab_id: u64, pi: usize, gen: u64, res: Result<Vec<Entry>, String>) {
        let t0 = Instant::now();
        let Some(ti) = self.tab_index(tab_id) else { return };
        {
            let p = &mut self.tabs[ti].panes[pi];
            if p.gen != gen {
                return;
            }
            p.loading = false;
            match res {
                Ok(v) => {
                    p.entries = v;
                    p.error = None;
                }
                Err(e) => {
                    p.entries.clear();
                    p.error = Some(e);
                }
            }
            // keep selection only for items that still exist
            let existing: HashSet<&PathBuf> = p.entries.iter().map(|e| &e.path).collect();
            p.selected.retain(|x| existing.contains(x));
            if !p.pending_select.is_empty() {
                p.selected = p.pending_select.drain(..).filter(|x| existing.contains(x)).collect();
            }
        }
        self.rebuild(ti, pi);
        if std::env::var("SKJ_TIMING").is_ok() {
            eprintln!("[skj-files] {} entries shown, rebuild took {:?}", self.tabs[ti].panes[pi].view.len(), t0.elapsed());
        }
        // put the cursor on the (first) selected item and scroll to it
        let first_sel = {
            let p = &self.tabs[ti].panes[pi];
            p.view.iter().position(|&i| p.selected.contains(&p.entries[i].path))
        };
        if let Some(idx) = first_sel {
            let p = &mut self.tabs[ti].panes[pi];
            if p.cursor.is_none() || p.cursor.map(|c| c >= p.view.len()).unwrap_or(false) {
                p.cursor = Some(idx);
                p.anchor = Some(idx);
            }
            if ti == self.cur {
                self.ensure_visible(pi, idx as i32);
            }
        }
        if ti == self.cur && std::mem::take(&mut self.select_first[pi]) {
            let p = &mut self.tabs[ti].panes[pi];
            if p.selected.is_empty() {
                if let Some(&i0) = p.view.first() {
                    let path = p.entries[i0].path.clone();
                    p.selected.insert(path);
                    p.cursor = Some(0);
                    p.anchor = Some(0);
                    self.refresh_selection_flags(ti, pi);
                }
            }
        }
        if ti == self.cur {
            self.sync_panes_flags();
            self.sync_status();
            self.sync_commands();
            self.update_preview();
            self.request_thumbs();
            self.refresh_columns(pi);
            self.refresh_child_column(pi);
            // a just-created "New folder" goes straight into rename
            if let Some(want) = self.rename_after_list.clone() {
                if self.tabs[ti].panes[pi].entries.iter().any(|e| e.path == want) {
                    self.rename_after_list = None;
                    self.tabs[ti].active = pi;
                    self.begin_rename(pi, want);
                }
            }
        }
    }

    fn on_search_batch(&mut self, tab_id: u64, pi: usize, gen: u64, batch: Vec<Entry>, scanned: usize, done: bool) {
        let Some(ti) = self.tab_index(tab_id) else { return };
        {
            let p = &mut self.tabs[ti].panes[pi];
            if p.gen != gen {
                return;
            }
            p.entries.extend(batch);
            p.searched += scanned;
            if done {
                p.search_done = true;
                p.loading = false;
            }
        }
        self.rebuild(ti, pi);
        if ti == self.cur {
            self.sync_panes_flags();
            self.sync_status();
            if self.tabs[ti].panes[pi].selected.is_empty() {
                self.update_preview();
            }
            if done {
                self.request_thumbs();
            }
        }
    }

    /// Thumbnails (grid view) and real app icons (also in the details list), made in the background.
    fn request_thumbs(&mut self) {
        let grid = self.s.view_mode == 1;
        let size: u32 = if grid { if self.s.grid_size == 2 { 256 } else { 160 } } else { 48 };
        let t = &self.tabs[self.cur];
        let mut jobs: Vec<thumbs::Job> = Vec::new();
        'outer: for pi in 0..2 {
            if pi == 1 && !t.dual {
                continue;
            }
            let p = &t.panes[pi];
            if p.loc == Loc::Bin {
                continue;
            }
            for &i in &p.view {
                let e = &p.entries[i];
                let src = thumbs::source_for(e);
                if src == thumbs::Src::None || (!grid && !thumbs::is_icon_kind(e)) {
                    continue;
                }
                let want = if thumbs::is_icon_kind(e) && !grid { 48 } else { size };
                if let Some((have, mt, _)) = self.thumbs.get(&e.path) {
                    if *have >= want && *mt == e.modified {
                        continue;
                    }
                }
                jobs.push(thumbs::Job { path: e.path.clone(), src, size: want });
                if jobs.len() >= 800 {
                    break 'outer;
                }
            }
        }
        let gen = self.thumb_gen.fetch_add(1, Ordering::SeqCst) + 1;
        self.thumb_pool.request(gen, jobs);
    }

    fn on_thumb(&mut self, _gen: u64, path: PathBuf, size: u32, t: thumbs::Thumb) {
        let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&t.rgba, t.w, t.h);
        let img = slint::Image::from_rgba8(buf);
        if self.thumbs.len() > 3000 {
            self.thumbs.clear();
        }
        let mtime = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
        self.thumbs.insert(path.clone(), (size, mtime, img.clone()));
        let t = &self.tabs[self.cur];
        for p in &t.panes {
            if let Some(row) = p.view.iter().position(|&i| p.entries[i].path == path) {
                if let Some(mut r) = p.model.row_data(row) {
                    r.thumb = img.clone();
                    r.has_thumb = true;
                    p.model.set_row_data(row, r);
                }
            }
        }
    }

    /// Filter + sort entries into the pane's view and push rows to the UI model.
    fn rebuild(&mut self, ti: usize, pi: usize) {
        let show_hidden = self.s.show_hidden;
        let show_ext = self.s.show_ext;
        let key = self.s.sort_key;
        let asc = self.s.sort_asc;
        let cut: HashSet<PathBuf> = match &self.clip {
            Some((v, true)) => v.iter().cloned().collect(),
            _ => HashSet::new(),
        };
        let thumbs = &self.thumbs;
        let tags = &self.tags;
        let friendly = self.s.friendly_dates;
        let p = &mut self.tabs[ti].panes[pi];
        let f = p.filter.to_lowercase();
        let mut view: Vec<usize> = (0..p.entries.len())
            .filter(|&i| {
                let e = &p.entries[i];
                (show_hidden || !e.hidden) && (f.is_empty() || e.name.to_lowercase().contains(&f))
            })
            .collect();
        let entries = &p.entries;
        view.sort_by(|&a, &b| {
            use std::cmp::Ordering::*;
            let (ea, eb) = (&entries[a], &entries[b]);
            if ea.is_dir != eb.is_dir {
                return if ea.is_dir { Less } else { Greater };
            }
            let o = match key {
                1 => ea.modified.cmp(&eb.modified),
                5 => ea.created.cmp(&eb.created),
                2 => ea.ext.cmp(&eb.ext),
                3 => ea.size.cmp(&eb.size),
                4 => fsx::natural_cmp(&ea.location, &eb.location),
                _ => Equal,
            }
            .then_with(|| fsx::natural_cmp(&ea.name, &eb.name));
            if asc { o } else { o.reverse() }
        });
        let rows: Vec<FileRow> = view
            .iter()
            .map(|&i| {
                let e = &entries[i];
                let name = if !show_ext && !e.is_dir && !e.ext.is_empty() { fsx::split_name(&e.name, false).0 } else { e.name.clone() };
                FileRow {
                    name: ss(name),
                    kind: e.kind,
                    size: if e.is_dir { SharedString::default() } else { ss(fsx::fmt_size(e.size)) },
                    date: ss(if friendly { fsx::friendly_time(e.modified) } else { fsx::fmt_time(e.modified) }),
                    created: ss(if friendly { fsx::friendly_time(e.created) } else { fsx::fmt_time(e.created) }),
                    type_name: ss(fsx::type_name(e)),
                    location: ss(&e.location),
                    selected: p.selected.contains(&e.path),
                    dim: e.hidden || cut.contains(&e.path),
                    thumb: thumbs.get(&e.path).filter(|t| t.1 == e.modified).map(|t| t.2.clone()).unwrap_or_default(),
                    has_thumb: thumbs.get(&e.path).map(|t| t.1 == e.modified).unwrap_or(false),
                    tags: match tags.get(&e.path) {
                        Some(&m) if m != 0 => ModelRc::new(VecModel::from((0..6).filter(|i| m & (1 << i) != 0).map(tag_color).collect::<Vec<_>>())),
                        _ => ModelRc::default(),
                    },
                }
            })
            .collect();
        p.view = view;
        p.model.set_vec(rows);
        if let Some(c) = p.cursor {
            if c >= p.view.len() {
                p.cursor = if p.view.is_empty() { None } else { Some(p.view.len() - 1) };
            }
        }
        // the row being renamed may have moved
        if ti == self.cur {
            if let Some((rp, path)) = &self.renaming {
                if *rp == pi {
                    let p = &self.tabs[ti].panes[pi];
                    let idx = p.view.iter().position(|&i| &p.entries[i].path == path).map(|x| x as i32).unwrap_or(-1);
                    self.g().set_rename_index(idx);
                }
            }
        }
    }

    fn rebuild_all(&mut self) {
        for ti in 0..self.tabs.len() {
            for pi in 0..2 {
                self.rebuild(ti, pi);
            }
        }
        self.sync_status();
        self.sync_commands();
        self.update_preview();
    }

    fn refresh_selection_flags(&mut self, ti: usize, pi: usize) {
        let p = &self.tabs[ti].panes[pi];
        for (row_i, &ei) in p.view.iter().enumerate() {
            let want = p.selected.contains(&p.entries[ei].path);
            if let Some(mut r) = p.model.row_data(row_i) {
                if r.selected != want {
                    r.selected = want;
                    p.model.set_row_data(row_i, r);
                }
            }
        }
    }

    // ───────────── selection ─────────────

    fn activate_pane(&mut self, pi: usize) {
        let t = &mut self.tabs[self.cur];
        if pi == 1 && !t.dual {
            return;
        }
        if t.active != pi {
            t.active = pi;
            self.sync_all();
        }
    }

    fn row_down(&mut self, pi: usize, idx: usize, ctrl: bool, shift: bool) {
        self.activate_pane(pi);
        let ti = self.cur;
        self.pending_collapse = None;
        // pressing on an already-selected item keeps the selection (so several files can be dragged);
        // it collapses to this one item on release if no drag happened
        if !ctrl && !shift {
            let p = &mut self.tabs[ti].panes[pi];
            if let Some(path) = p.entry(idx).map(|e| e.path.clone()) {
                if p.selected.len() > 1 && p.selected.contains(&path) {
                    p.cursor = Some(idx);
                    self.pending_collapse = Some((pi, idx));
                    self.ui.invoke_focus_main();
                    return;
                }
            }
        }
        {
            let p = &mut self.tabs[ti].panes[pi];
            if idx >= p.view.len() {
                return;
            }
            let path = p.entries[p.view[idx]].path.clone();
            if shift {
                let a = p.anchor.unwrap_or(idx);
                if !ctrl {
                    p.selected.clear();
                }
                let (lo, hi) = if a <= idx { (a, idx) } else { (idx, a) };
                for i in lo..=hi {
                    let pp = p.entries[p.view[i]].path.clone();
                    p.selected.insert(pp);
                }
            } else if ctrl {
                if !p.selected.remove(&path) {
                    p.selected.insert(path);
                }
                p.anchor = Some(idx);
            } else {
                p.selected.clear();
                p.selected.insert(path);
                p.anchor = Some(idx);
            }
            p.cursor = Some(idx);
        }
        self.refresh_selection_flags(ti, pi);
        self.after_selection();
        self.ui.invoke_focus_main();
    }

    fn row_click(&mut self, pi: usize, idx: usize) {
        if self.pending_collapse.take() == Some((pi, idx)) {
            let ti = self.cur;
            let p = &mut self.tabs[ti].panes[pi];
            if let Some(path) = p.entry(idx).map(|e| e.path.clone()) {
                p.selected.clear();
                p.selected.insert(path);
                p.anchor = Some(idx);
            }
            self.refresh_selection_flags(ti, pi);
            self.after_selection();
        }
    }

    fn bg_down(&mut self, pi: usize, keep: bool) {
        self.activate_pane(pi);
        let ti = self.cur;
        if keep {
            self.ui.invoke_focus_main();
            return;
        }
        self.tabs[ti].panes[pi].selected.clear();
        self.refresh_selection_flags(ti, pi);
        self.after_selection();
        self.ui.invoke_focus_main();
    }

    fn select_all(&mut self) {
        let ti = self.cur;
        let pi = self.ap();
        {
            let p = &mut self.tabs[ti].panes[pi];
            let all: Vec<PathBuf> = p.view.iter().map(|&i| p.entries[i].path.clone()).collect();
            p.selected = all.into_iter().collect();
        }
        self.refresh_selection_flags(ti, pi);
        self.after_selection();
    }

    // ── selection box ──
    fn marquee_begin(&mut self, pi: usize, additive: bool) {
        self.activate_pane(pi);
        self.pending_collapse = None;
        let p = &self.tabs[self.cur].panes[pi];
        self.marquee_base = if additive { p.selected.clone() } else { HashSet::new() };
    }

    /// Select everything the box touches. Coordinates are in list-content pixels; the layout is
    /// `cols` cells of `cell_w`×`cell_h` (items `item_w`×`item_h`), starting `pad` from the left.
    #[allow(clippy::too_many_arguments)]
    fn marquee_update(&mut self, pi: usize, x0: f32, y0: f32, x1: f32, y1: f32, cols: i32, cell_w: f32, cell_h: f32, item_w: f32, item_h: f32, pad: f32) {
        let ti = self.cur;
        let (l, r) = (x0.min(x1), x0.max(x1));
        let (t, b) = (y0.min(y1).max(0.0), y0.max(y1).max(0.0));
        let cols = cols.max(1) as usize;
        let p = &mut self.tabs[ti].panes[pi];
        let n = p.view.len();
        let mut hit: Vec<usize> = Vec::new();
        if cell_h > 0.0 && n > 0 {
            let r0 = (t / cell_h).floor() as usize;
            let r1 = (b / cell_h).floor() as usize;
            for row in r0..=r1 {
                let ry = row as f32 * cell_h;
                if ry + item_h < t || ry > b {
                    continue;
                }
                for c in 0..cols {
                    let idx = row * cols + c;
                    if idx >= n {
                        break;
                    }
                    if cols > 1 {
                        let cx = pad + c as f32 * cell_w;
                        if cx + item_w < l || cx > r {
                            continue;
                        }
                    }
                    hit.push(idx);
                }
                if row * cols >= n {
                    break;
                }
            }
        }
        let mut sel = self.marquee_base.clone();
        for &i in &hit {
            sel.insert(p.entries[p.view[i]].path.clone());
        }
        if let Some(&last) = hit.last() {
            p.cursor = Some(last);
            if p.anchor.is_none() {
                p.anchor = hit.first().copied();
            }
        }
        if sel != p.selected {
            p.selected = sel;
            self.refresh_selection_flags(ti, pi);
            self.sync_status();
            self.sync_commands();
        }
    }

    fn marquee_end(&mut self) {
        self.marquee_base.clear();
        self.update_preview();
        self.ui.invoke_focus_main();
    }

    fn invert_selection(&mut self) {
        let ti = self.cur;
        let pi = self.ap();
        {
            let p = &mut self.tabs[ti].panes[pi];
            let all: Vec<PathBuf> = p.view.iter().map(|&i| p.entries[i].path.clone()).collect();
            let new: HashSet<PathBuf> = all.into_iter().filter(|x| !p.selected.contains(x)).collect();
            p.selected = new;
        }
        self.refresh_selection_flags(ti, pi);
        self.after_selection();
    }

    fn clear_selection(&mut self) {
        let ti = self.cur;
        let pi = self.ap();
        self.tabs[ti].panes[pi].selected.clear();
        self.refresh_selection_flags(ti, pi);
        self.after_selection();
    }

    fn after_selection(&mut self) {
        self.sync_status();
        self.sync_commands();
        self.update_preview();
        let pi = self.ap();
        self.refresh_child_column(pi);
    }

    fn move_cursor(&mut self, delta: i64, shift: bool, absolute: Option<usize>) {
        let ti = self.cur;
        let pi = self.ap();
        let n = self.tabs[ti].panes[pi].view.len();
        if n == 0 {
            return;
        }
        let new = {
            let p = &mut self.tabs[ti].panes[pi];
            let cur = p.cursor.map(|c| c as i64).unwrap_or(-1);
            let target = match absolute {
                Some(a) => a as i64,
                None => {
                    if cur < 0 { if delta >= 0 { 0 } else { n as i64 - 1 } } else { cur + delta }
                }
            };
            let target = target.clamp(0, n as i64 - 1) as usize;
            if shift {
                let a = p.anchor.unwrap_or(cur.max(0) as usize);
                p.selected.clear();
                let (lo, hi) = if a <= target { (a, target) } else { (target, a) };
                for i in lo..=hi {
                    let pp = p.entries[p.view[i]].path.clone();
                    p.selected.insert(pp);
                }
                p.anchor = Some(a);
            } else {
                p.selected.clear();
                let pp = p.entries[p.view[target]].path.clone();
                p.selected.insert(pp);
                p.anchor = Some(target);
            }
            p.cursor = Some(target);
            target
        };
        self.refresh_selection_flags(ti, pi);
        self.ensure_visible(pi, new as i32);
        self.after_selection();
    }

    fn ensure_visible(&mut self, pi: usize, idx: i32) {
        self.ensure_gen += 1;
        let g = self.g();
        g.set_ensure_pane(pi as i32);
        g.set_ensure_index(idx);
        g.set_ensure_gen(self.ensure_gen);
    }

    fn type_ahead(&mut self, ch: &str) {
        if self.type_at.elapsed() > Duration::from_millis(900) {
            self.type_buf.clear();
        }
        self.type_at = Instant::now();
        self.type_buf.push_str(&ch.to_lowercase());
        let buf = self.type_buf.clone();
        let p = self.pane();
        let start = p.cursor.map(|c| if buf.chars().count() == 1 { c + 1 } else { c }).unwrap_or(0);
        let n = p.view.len();
        let found = (0..n).map(|k| (start + k) % n.max(1)).find(|&i| p.entries[p.view[i]].name.to_lowercase().starts_with(&buf));
        if let Some(i) = found {
            self.move_cursor(0, false, Some(i));
        }
    }

    // ───────────── opening ─────────────

    fn open_index(&mut self, pi: usize, idx: usize, new_tab: bool) {
        if self.tabs[self.cur].panes[pi].loc == Loc::Bin {
            self.toast(ts("Restore it first to open it (right-click → Restore)"));
            return;
        }
        let Some(e) = self.tabs[self.cur].panes[pi].entry(idx).cloned() else { return };
        if e.is_dir {
            if new_tab {
                self.open_tab(Loc::Dir(e.path), false);
                self.toast(tf!("Opened \"{}\" in a new tab", e.name));
            } else {
                self.navigate(pi, Loc::Dir(e.path), true);
            }
        } else if let Err(err) = fsx::open_file(&e.path) {
            self.toast(tf!("Couldn't open {}: {}", e.name, err));
        } else {
            self.remember_file(e.path.clone());
        }
    }

    fn open_selected(&mut self) {
        if self.pane().loc == Loc::Bin {
            self.toast(ts("Restore it first to open it (right-click → Restore)"));
            return;
        }
        let pi = self.ap();
        let sel = self.pane().selected_entries().into_iter().cloned().collect::<Vec<_>>();
        if sel.len() == 1 && sel[0].is_dir {
            self.navigate(pi, Loc::Dir(sel[0].path.clone()), true);
            return;
        }
        for e in sel.iter().take(20) {
            if e.is_dir {
                self.open_tab(Loc::Dir(e.path.clone()), false);
            } else if let Err(err) = fsx::open_file(&e.path) {
                self.toast(tf!("Couldn't open {}: {}", e.name, err));
            } else {
                self.remember_file(e.path.clone());
            }
        }
    }

    // ───────────── sync to UI ─────────────

    fn sync_all(&mut self) {
        self.refresh_titles();
        self.sync_panes_models();
        self.sync_panes_flags();
        self.sync_nav();
        self.sync_status();
        self.sync_commands();
        self.update_preview();
    }

    fn loc_title(&self, loc: &Loc) -> (String, i32) {
        match loc {
            Loc::Home => (ts("Home").into(), fsx::K_HOME),
            Loc::Search { query, .. } => (tf!("Search: {}", query), fsx::K_SEARCH),
            Loc::Tag(i) => (tf!("{} tag", ts(TAG_NAMES[*i as usize % 6])), 27),
            Loc::Bin => (capitalize(fsx::trash_word()), fsx::K_TRASH),
            Loc::Dir(p) => {
                if p.parent().is_none() {
                    let label = self.drives.iter().find(|d| &d.mount == p).map(|d| d.label.clone()).unwrap_or_else(|| p.display().to_string());
                    (label, fsx::K_DRIVE)
                } else {
                    (fsx::file_name(p), fsx::kind_for_place(p))
                }
            }
        }
    }

    fn refresh_titles(&mut self) {
        let tabs: Vec<TabInfo> = self
            .tabs
            .iter()
            .map(|t| {
                let (title, kind) = self.loc_title(&t.panes[t.active].loc);
                TabInfo { title: ss(title), kind }
            })
            .collect();
        self.sync_tab_drags();
        // update in place: replacing the model would rebuild the tab strip under the mouse
        // (and cancel a tab drag that just started)
        let m = self.tabs_model.clone();
        if m.row_count() == tabs.len() {
            for (i, t) in tabs.into_iter().enumerate() {
                if m.row_data(i).as_ref() != Some(&t) {
                    m.set_row_data(i, t);
                }
            }
        } else {
            m.set_vec(tabs);
        }
        let g = self.g();
        g.set_active_tab(self.cur as i32);
        let (title, _) = self.loc_title(&self.pane().loc);
        g.set_window_title(ss(format!("{} — SKJ Files", title)));
    }

    fn sync_panes_models(&self) {
        let t = &self.tabs[self.cur];
        let g = self.g();
        g.set_p0_files(ModelRc::from(t.panes[0].model.clone()));
        g.set_p1_files(ModelRc::from(t.panes[1].model.clone()));
    }

    fn sync_panes_flags(&self) {
        let t = &self.tabs[self.cur];
        let g = self.g();
        g.set_dual(t.dual);
        g.set_active_pane(t.active as i32);
        for pi in 0..2 {
            let p = &t.panes[pi];
            let home = p.loc == Loc::Home;
            let search = matches!(p.loc, Loc::Search { .. } | Loc::Tag(_) | Loc::Bin);
            let empty = if let Some(e) = &p.error {
                e.clone()
            } else if p.loc == Loc::Bin {
                tf!("The {} is empty", fsx::trash_word())
            } else if let Loc::Tag(_) = p.loc {
                ts("No files have this tag yet — right-click a file to tag it").into()
            } else if search {
                if p.search_done { ts("Nothing found").into() } else { ts("Searching…").into() }
            } else if !p.filter.is_empty() {
                tf!("No items match \"{}\"", p.filter)
            } else {
                ts("This folder is empty").into()
            };
            let header = match &p.loc {
                Loc::Home => ts("Home").into(),
                Loc::Dir(d) => d.display().to_string(),
                Loc::Search { root, query } => tf!("Search \"{}\" in {}", query, root.display()),
                Loc::Tag(i) => tf!("{} tag", ts(TAG_NAMES[*i as usize % 6])),
                Loc::Bin => capitalize(fsx::trash_word()),
            };
            let bin = p.loc == Loc::Bin;
            if pi == 0 {
                g.set_p0_bin(bin);
                g.set_p0_home(home);
                g.set_p0_search(search);
                g.set_p0_loading(p.loading && p.entries.is_empty() && p.error.is_none());
                g.set_p0_empty(ss(empty));
                g.set_p0_header(ss(header));
            } else {
                g.set_p1_bin(bin);
                g.set_p1_home(home);
                g.set_p1_search(search);
                g.set_p1_loading(p.loading && p.entries.is_empty() && p.error.is_none());
                g.set_p1_empty(ss(empty));
                g.set_p1_header(ss(header));
            }
        }
    }

    fn sync_nav(&mut self) {
        let loc = self.pane().loc.clone();
        let (crumbs, paths, kind, path_text): (Vec<String>, Vec<PathBuf>, i32, String) = match &loc {
            Loc::Home => (vec![ts("Home").into()], vec![], fsx::K_HOME, String::new()),
            Loc::Tag(i) => (vec![ts("Tags").into(), ts(TAG_NAMES[*i as usize % 6]).into()], vec![], 27, String::new()),
            Loc::Bin => (vec![capitalize(fsx::trash_word())], vec![], fsx::K_TRASH, String::new()),
            Loc::Search { root, query } => (
                vec![fsx::file_name(root), tf!("Search results for \"{}\"", query)],
                vec![root.clone(), root.clone()],
                fsx::K_SEARCH,
                root.display().to_string(),
            ),
            Loc::Dir(p) => {
                let mut chain: Vec<PathBuf> = p.ancestors().map(|a| a.to_path_buf()).collect();
                chain.reverse();
                let labels = chain
                    .iter()
                    .map(|a| {
                        if a.parent().is_none() {
                            let s = a.display().to_string();
                            if cfg!(windows) { s.trim_end_matches('\\').to_string() } else { "/".to_string() }
                        } else {
                            fsx::file_name(a)
                        }
                    })
                    .collect();
                let kind = if p.parent().is_none() { fsx::K_DRIVE } else { fsx::kind_for_place(p) };
                (labels, chain, kind, p.display().to_string())
            }
        };
        self.crumb_paths = paths;
        let p = self.pane();
        let g = self.g();
        g.set_crumbs(ModelRc::new(VecModel::from(crumbs.into_iter().enumerate().map(|(i, l)| Crumb { label: ss(l), index: i as i32 }).collect::<Vec<_>>())));
        g.set_loc_kind(kind);
        g.set_path_text(ss(path_text));
        g.set_can_back(!p.back.is_empty());
        g.set_can_forward(!p.fwd.is_empty());
        g.set_can_up(matches!(&loc, Loc::Dir(_) | Loc::Search { .. } | Loc::Tag(_) | Loc::Bin));
        g.set_in_bin(loc == Loc::Bin);
        g.set_git_branch(ss(loc.dir().and_then(fsx::git_branch).unwrap_or_default()));
        g.set_loc_tag(if let Loc::Tag(i) = &loc { *i as i32 } else { -1 });
        g.set_in_folder(matches!(&loc, Loc::Dir(_)));
        g.set_search_text(ss(&p.filter));
        let (title, _) = self.loc_title(&loc);
        g.set_search_placeholder(ss(match &loc {
            Loc::Home => ts("Search your home folder").to_string(),
            Loc::Search { .. } | Loc::Tag(_) | Loc::Bin => ts("Filter these results").to_string(),
            _ => tf!("Search {}", title),
        }));
        g.set_side_active(ss(match &loc {
            Loc::Home => "::home".to_string(),
            Loc::Dir(d) => d.display().to_string(),
            Loc::Search { .. } => String::new(),
            Loc::Tag(i) => format!("::tag:{}", i),
            Loc::Bin => "::bin".to_string(),
        }));
        g.set_edit_path(false);
    }

    fn sync_sidebar(&self) {
        let g = self.g();
        let quick: Vec<SideItem> = self
            .quick
            .iter()
            .map(|(l, k, p)| SideItem { label: ss(i18n::place_label(l)), kind: *k, path: ss(p.display().to_string()), usage: -1.0, sub: ss(p.display().to_string()), color: Color::default() })
            .collect();
        g.set_quick(ModelRc::new(VecModel::from(quick)));
        let drives: Vec<SideItem> = self
            .drives
            .iter()
            .filter(|d| !d.network)
            .map(|d| SideItem {
                label: ss(&d.label),
                kind: if d.removable { fsx::K_USB } else { fsx::K_DRIVE },
                path: ss(d.mount.display().to_string()),
                usage: if d.total > 0 { (d.total - d.free.min(d.total)) as f32 / d.total as f32 } else { -1.0 },
                sub: ss(if d.total > 0 { tf!("{} free of {}", fsx::fmt_size(d.free), fsx::fmt_size(d.total)) } else { String::new() }),
                color: Color::default(),
            })
            .collect();
        g.set_drives(ModelRc::new(VecModel::from(drives)));
        let mut net: Vec<SideItem> = self
            .drives
            .iter()
            .filter(|d| d.network)
            .map(|d| SideItem { label: ss(&d.label), kind: 28, path: ss(d.mount.display().to_string()), usage: -1.0, sub: ss(d.mount.display().to_string()), color: Color::default() })
            .collect();
        for (i, srv) in self.s.servers.iter().enumerate() {
            net.push(SideItem { label: ss(fsx::server_label(srv)), kind: 28, path: ss(format!("::server:{}", i)), usage: -1.0, sub: ss(srv), color: Color::default() });
        }
        net.push(SideItem { label: ss(ts("Connect to server…")), kind: 29, path: ss("::connect"), usage: -1.0, sub: ss(""), color: Color::default() });
        g.set_network(ModelRc::new(VecModel::from(net)));
        let recent: Vec<SideItem> = self
            .s
            .recent
            .iter()
            .filter(|p| p.is_dir())
            .take(8)
            .map(|p| SideItem { label: ss(fsx::file_name(p)), kind: 0, path: ss(p.display().to_string()), usage: -1.0, sub: ss(p.display().to_string()), color: Color::default() })
            .collect();
        g.set_recent(ModelRc::new(VecModel::from(recent)));
        let mut places = Vec::new();
        if fsx::BIN_VIEW {
            places.push(SideItem { label: ss(capitalize(fsx::trash_word())), kind: fsx::K_TRASH, path: ss("::bin"), usage: -1.0, sub: ss(""), color: Color::default() });
        }
        g.set_places(ModelRc::new(VecModel::from(places)));
        let mut tag_items = Vec::new();
        for i in 0..6usize {
            let n = self.tags.values().filter(|m| **m & (1 << i) != 0).count();
            if n > 0 {
                tag_items.push(SideItem { label: ss(ts(TAG_NAMES[i])), kind: 27, path: ss(format!("::tag:{}", i)), usage: -1.0, sub: ss(n.to_string()), color: tag_color(i) });
            }
        }
        g.set_tag_items(ModelRc::new(VecModel::from(tag_items)));
    }

    fn sync_status(&self) {
        let p = self.pane();
        let g = self.g();
        let left = match &p.loc {
            Loc::Home => tf!("{} drives", self.drives.len()),
            _ => {
                let n = p.view.len();
                let mut s = if let Loc::Tag(_) = p.loc {
                    tf!("{} tagged item{}", fsx::fmt_count(n), if n == 1 { "" } else { "s" })
                } else if let Loc::Search { .. } = p.loc {
                    if p.search_done {
                        tf!("{} results", fsx::fmt_count(n))
                    } else {
                        tf!("Searching… {} found · {} checked", fsx::fmt_count(n), fsx::fmt_count(p.searched))
                    }
                } else {
                    tf!("{} item{}", fsx::fmt_count(n), if n == 1 { "" } else { "s" })
                };
                let sel = p.selected_entries();
                if !sel.is_empty() {
                    s.push_str(&tf!("  ·  {} selected", fsx::fmt_count(sel.len())));
                    let bytes: u64 = sel.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
                    if bytes > 0 {
                        s.push_str(&format!("  ·  {}", fsx::fmt_size(bytes)));
                    }
                }
                s
            }
        };
        g.set_status_left(ss(left));
        let right = match p.loc.dir() {
            Some(d) => self.drive_for(d).filter(|dr| dr.total > 0).map(|dr| tf!("{} free on {}", fsx::fmt_size(dr.free), dr.label)).unwrap_or_default(),
            None => String::new(),
        };
        g.set_status_right(ss(right));
    }

    fn drive_for(&self, d: &Path) -> Option<&Drive> {
        let ds = d.display().to_string().to_lowercase();
        self.drives
            .iter()
            .filter(|dr| {
                let m = dr.mount.display().to_string().to_lowercase();
                ds.starts_with(&m)
            })
            .max_by_key(|dr| dr.mount.as_os_str().len())
    }

    fn sync_commands(&self) {
        let p = self.pane();
        let g = self.g();
        g.set_sel_count(p.selected.len() as i32);
        g.set_can_paste(self.clip.is_some() || self.system_clipboard_has_files());
        g.set_can_undo(!self.undo.is_empty());
        // Slint only tracks a press for dragging when the payload is non-empty at press time,
        // and the pressed row becomes the selection right after — so never leave it empty.
        let mut dt = DataTransfer::default();
        let paths = p.selected_paths();
        if paths.is_empty() {
            dt.set_plain_text(SharedString::from("skj-files"));
        } else {
            dt.set_file_paths(paths);
        }
        g.set_drag_data(dt);
    }

    // ───────────── preview ─────────────

    fn update_preview(&mut self) {
        let gen = self.pv_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let g = self.g();
        g.set_pv_has_image(false);
        g.set_pv_has_text(false);
        g.set_pv_text(SharedString::default());
        let p = self.pane();
        let mut props: Vec<(String, String)> = Vec::new();
        match &p.loc {
            Loc::Home => {
                g.set_pv_kind(fsx::K_HOME);
                g.set_pv_title(ss(ts("Home")));
                g.set_pv_subtitle(ss(ts("Quick access, drives and recent folders")));
            }
            _ => {
                let sel = p.selected_entries();
                if p.loc == Loc::Bin && sel.len() == 1 {
                    // a deleted item: no content preview, just where it came from
                    let e = sel[0];
                    g.set_pv_kind(e.kind);
                    g.set_pv_title(ss(&e.name));
                    g.set_pv_subtitle(ss(fsx::type_name(e)));
                    if !e.is_dir {
                        props.push((ts("Size").into(), fsx::fmt_size(e.size)));
                    }
                    props.push((ts("Deleted").into(), fsx::fmt_time(e.modified)));
                    props.push((ts("From").into(), e.location.clone()));
                } else if sel.is_empty() {
                    let (title, kind) = self.loc_title(&p.loc);
                    g.set_pv_kind(if kind == fsx::K_SEARCH || kind == fsx::K_TRASH { kind } else { fsx::K_FOLDER });
                    g.set_pv_title(ss(title));
                    let dirs = p.view.iter().filter(|&&i| p.entries[i].is_dir).count();
                    let files = p.view.len() - dirs;
                    g.set_pv_subtitle(ss(format!("{} · {}", plural(dirs, "folder"), plural(files, "file"))));
                    if let Some(d) = p.loc.dir() {
                        props.push((ts("Location").into(), d.display().to_string()));
                    }
                } else if sel.len() > 1 {
                    g.set_pv_kind(if sel.iter().all(|e| e.is_dir) { fsx::K_FOLDER } else { fsx::K_FILE });
                    g.set_pv_title(ss(tf!("{} items selected", fsx::fmt_count(sel.len()))));
                    let bytes: u64 = sel.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
                    let nd = sel.iter().filter(|e| e.is_dir).count();
                    g.set_pv_subtitle(ss(format!("{} · {} · {}", plural(nd, "folder"), plural(sel.len() - nd, "file"), fsx::fmt_size(bytes))));
                } else {
                    let e = sel[0].clone();
                    g.set_pv_kind(e.kind);
                    g.set_pv_title(ss(&e.name));
                    g.set_pv_subtitle(ss(fsx::type_name(&e)));
                    if !e.is_dir {
                        props.push((ts("Size").into(), tf!("{} ({} bytes)", fsx::fmt_size(e.size), fsx::fmt_count(e.size as usize))));
                    }
                    props.push((ts("Modified").into(), fsx::fmt_time(e.modified)));
                    if let Ok(md) = std::fs::metadata(&e.path) {
                        if let Ok(c) = md.created() {
                            props.push((ts("Created").into(), fsx::fmt_time(Some(c))));
                        }
                    }
                    props.push((ts("Location").into(), e.path.parent().map(|x| x.display().to_string()).unwrap_or_default()));
                    self.spawn_preview(gen, e);
                }
            }
        }
        let rows: Vec<PropRow> = props.into_iter().map(|(k, v)| PropRow { key: ss(k), value: ss(v) }).collect();
        g.set_pv_props(ModelRc::new(VecModel::from(rows)));
    }

    fn spawn_preview(&self, gen: u64, e: Entry) {
        if !self.s.show_preview {
            return;
        }
        let cur = self.pv_gen.clone();
        let is_img = matches!(e.ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "jfif");
        let is_text = matches!(e.kind, fsx::K_CODE | fsx::K_TEXT | fsx::K_SHEET)
            && !matches!(e.ext.as_str(), "doc" | "docx" | "odt" | "pages" | "epub" | "ppt" | "pptx" | "odp" | "key" | "xls" | "xlsx" | "ods" | "numbers" | "rtf")
            || (e.ext.is_empty() && !e.is_dir && e.size < 256 * 1024);
        if e.is_dir {
            std::thread::spawn(move || {
                let (mut nf, mut nd) = (0usize, 0usize);
                if let Ok(rd) = std::fs::read_dir(&e.path) {
                    for x in rd.flatten() {
                        if cur.load(Ordering::SeqCst) != gen {
                            return;
                        }
                        if x.file_type().map(|t| t.is_dir()).unwrap_or(false) { nd += 1 } else { nf += 1 }
                    }
                }
                let _ = slint::invoke_from_event_loop(move || with_app(move |a| a.on_preview_prop(gen, ts("Contains"), tf!("{} folders, {} files", fsx::fmt_count(nd), fsx::fmt_count(nf)))));
            });
        } else if is_img && e.size < 80 * 1024 * 1024 {
            std::thread::spawn(move || {
                let Ok(img) = image::open(&e.path) else { return };
                if cur.load(Ordering::SeqCst) != gen {
                    return;
                }
                let (w, h) = (img.width(), img.height());
                let t = img.thumbnail(640, 640).to_rgba8();
                let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(t.as_raw(), t.width(), t.height());
                let _ = slint::invoke_from_event_loop(move || with_app(move |a| a.on_preview_image(gen, buf, w, h)));
            });
        } else if is_text && e.size < 8 * 1024 * 1024 {
            std::thread::spawn(move || {
                use std::io::Read;
                let Ok(mut f) = std::fs::File::open(&e.path) else { return };
                let mut buf = vec![0u8; 24 * 1024];
                let n = f.read(&mut buf).unwrap_or(0);
                buf.truncate(n);
                if buf.iter().take(4096).any(|&b| b == 0) {
                    return;
                }
                let s = String::from_utf8_lossy(&buf);
                let text: String = s
                    .lines()
                    .take(80)
                    .map(|l| {
                        let l = l.replace('\t', "    ");
                        if l.chars().count() > 160 { l.chars().take(160).collect::<String>() + "…" } else { l }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let _ = slint::invoke_from_event_loop(move || with_app(move |a| a.on_preview_text(gen, text)));
            });
        }
    }

    fn on_preview_image(&mut self, gen: u64, buf: SharedPixelBuffer<Rgba8Pixel>, w: u32, h: u32) {
        if self.pv_gen.load(Ordering::SeqCst) != gen {
            return;
        }
        let g = self.g();
        g.set_pv_image(slint::Image::from_rgba8(buf));
        g.set_pv_has_image(true);
        self.on_preview_prop(gen, ts("Dimensions"), format!("{} × {}", w, h));
    }

    fn on_preview_text(&mut self, gen: u64, text: String) {
        if self.pv_gen.load(Ordering::SeqCst) != gen {
            return;
        }
        let g = self.g();
        g.set_pv_text(ss(text));
        g.set_pv_has_text(true);
    }

    fn on_preview_prop(&mut self, gen: u64, key: &str, value: String) {
        if self.pv_gen.load(Ordering::SeqCst) != gen {
            return;
        }
        let g = self.g();
        let m = g.get_pv_props();
        let mut rows: Vec<PropRow> = (0..m.row_count()).filter_map(|i| m.row_data(i)).collect();
        rows.insert(rows.len().min(1), PropRow { key: ss(key), value: ss(value) });
        g.set_pv_props(ModelRc::new(VecModel::from(rows)));
    }

    // ───────────── toast ─────────────

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast_gen += 1;
        let id = self.toast_gen;
        self.g().set_toast(ss(i18n::t(&msg.into())));
        slint::Timer::single_shot(Duration::from_millis(3200), move || {
            with_app(move |a| {
                if a.toast_gen == id {
                    a.g().set_toast(SharedString::default());
                }
            })
        });
    }

    // ───────────── clipboard ─────────────

    fn system_clipboard_has_files(&self) -> bool {
        #[cfg(windows)]
        {
            let seq = clipboard_win::raw::seq_num().map(|n| n.get()).unwrap_or(0);
            if seq != self.clip_seq {
                return clipboard_win::is_format_avail(clipboard_win::formats::CF_HDROP);
            }
        }
        false
    }

    fn set_clip(&mut self, paths: Vec<PathBuf>, cut: bool) {
        #[cfg(windows)]
        {
            let strs: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
            if let Ok(_open) = clipboard_win::Clipboard::new_attempts(10) {
                let _ = clipboard_win::raw::empty();
                if clipboard_win::raw::set_file_list(&strs).is_ok() {
                    self.clip_seq = clipboard_win::raw::seq_num().map(|n| n.get()).unwrap_or(0);
                }
            }
        }
        let n = paths.len();
        self.clip = Some((paths, cut));
        self.toast(if cut { tf!("Cut {} item{}", n, if n == 1 { "" } else { "s" }) } else { tf!("Copied {} item{}", n, if n == 1 { "" } else { "s" }) });
        // refresh dimming for cut items
        let ti = self.cur;
        for pi in 0..2 {
            self.rebuild(ti, pi);
        }
        self.sync_commands();
    }

    fn take_paste_source(&mut self) -> Option<(Vec<PathBuf>, bool)> {
        #[cfg(windows)]
        {
            let seq = clipboard_win::raw::seq_num().map(|n| n.get()).unwrap_or(0);
            if seq != self.clip_seq {
                // someone else (e.g. Explorer) put files on the clipboard
                if let Ok(list) = clipboard_win::get_clipboard::<Vec<String>, _>(clipboard_win::formats::FileList) {
                    if !list.is_empty() {
                        return Some((list.into_iter().map(PathBuf::from).collect(), false));
                    }
                }
            }
        }
        self.clip.clone()
    }

    fn copy_text(&mut self, text: String) {
        if self.sys_clip.is_none() {
            self.sys_clip = arboard::Clipboard::new().ok();
        }
        match self.sys_clip.as_mut().map(|c| c.set_text(text)) {
            Some(Ok(_)) => self.toast(ts("Path copied")),
            _ => self.toast(ts("Couldn't access the clipboard")),
        }
    }

    // ───────────── file operations ─────────────

    fn start_op(&mut self, kind: OpKind, srcs: Vec<PathBuf>, dest: Option<PathBuf>) {
        self.start_op_ex(kind, srcs, dest, UndoHint::Normal);
    }

    fn start_op_ex(&mut self, kind: OpKind, srcs: Vec<PathBuf>, dest: Option<PathBuf>, hint: UndoHint) {
        if srcs.is_empty() {
            return;
        }
        let op = PendingOp { kind, srcs, dest, hint, policies: None };
        if self.op_running || self.conflict.is_some() {
            self.op_queue.push_back(op);
            self.toast(ts("Queued — another operation is running"));
            return;
        }
        self.run_pending(op);
    }

    fn pump_queue(&mut self) {
        if self.op_running || self.conflict.is_some() {
            return;
        }
        if let Some(op) = self.op_queue.pop_front() {
            self.run_pending(op);
        }
    }

    fn run_pending(&mut self, mut op: PendingOp) {
        // name clashes are decided before anything is touched
        if op.policies.is_none() && matches!(op.kind, OpKind::Copy | OpKind::Move) {
            if let Some(d) = op.dest.clone() {
                let list = fsx::find_conflicts(op.kind, &op.srcs, &d);
                if !list.is_empty() {
                    if !matches!(self.dialog, Dialog::None) {
                        // some other dialog is open: ask once it's closed
                        self.op_queue.push_front(op);
                        slint::Timer::single_shot(Duration::from_millis(400), || with_app(|a| a.pump_queue()));
                        return;
                    }
                    self.conflict = Some(ConflictState { op, list, idx: 0, decided: HashMap::new() });
                    self.show_conflict();
                    return;
                }
            }
        }
        let policies = op.policies.take().unwrap_or_default();
        let PendingOp { kind, srcs, dest, hint, .. } = op;
        self.op_running = true;
        self.op_cancel.store(false, Ordering::SeqCst);
        let cancel = self.op_cancel.clone();
        let g = self.g();
        g.set_op_progress(0.0);
        g.set_op_text(ss(ts("Starting…")));
        g.set_op_cancelling(false);
        std::thread::spawn(move || {
            let report = |f: f32, t: String| {
                let _ = slint::invoke_from_event_loop(move || with_app(move |a| a.on_op_progress(f, t)));
            };
            let res = fsx::run_op(kind, &srcs, dest.as_deref(), &policies, &cancel, &report);
            let _ = slint::invoke_from_event_loop(move || with_app(move |a| a.on_op_done(kind, srcs, dest, res, hint)));
        });
    }

    fn cancel_op(&mut self) {
        if self.op_running {
            self.op_cancel.store(true, Ordering::SeqCst);
            let g = self.g();
            g.set_op_cancelling(true);
            g.set_op_text(ss(ts("Cancelling…")));
        }
    }

    // ── name clash dialog ──
    fn show_conflict(&mut self) {
        let Some(c) = &self.conflict else { return };
        let (src, dst) = c.list[c.idx].clone();
        let left = c.list.len() - c.idx - 1;
        let kind = c.op.kind;
        let describe = |p: &Path| -> (String, Option<std::time::SystemTime>, u64, bool) {
            match std::fs::metadata(p) {
                Ok(m) => (
                    if m.is_dir() { ts("Folder").into() } else { fsx::fmt_size(m.len()) },
                    m.modified().ok(),
                    if m.is_dir() { 0 } else { m.len() },
                    m.is_dir(),
                ),
                Err(_) => ("?".into(), None, 0, false),
            }
        };
        let (ns, nt, nb, nd) = describe(&src);
        let (os, ot, ob, od) = describe(&dst);
        let badge = |mine: Option<std::time::SystemTime>, other: Option<std::time::SystemTime>, mb: u64, obb: u64, dir: bool| -> String {
            let mut v = Vec::new();
            if mine > other && other.is_some() {
                v.push(ts("Newer"));
            }
            if !dir && mb > obb {
                v.push(ts("Larger"));
            }
            v.join(" · ")
        };
        let where_ = dst.parent().map(fsx::file_name).unwrap_or_default();
        let g = self.g();
        g.set_cf_name(ss(fsx::file_name(&dst)));
        g.set_cf_kind(if nd { fsx::K_FOLDER } else { fsx::kind_for_ext(&fsx::ext_of(&fsx::file_name(&src))) });
        g.set_cf_new(ss(format!("{} · {}", ns, fsx::fmt_time(nt))));
        g.set_cf_old(ss(format!("{} · {}", os, fsx::fmt_time(ot))));
        g.set_cf_new_badge(ss(badge(nt, ot, nb, ob, nd)));
        g.set_cf_old_badge(ss(badge(ot, nt, ob, nb, od)));
        g.set_cf_new_from(ss(src.parent().map(|p| p.display().to_string()).unwrap_or_default()));
        g.set_cf_merge(nd && od);
        g.set_cf_left(left as i32);
        g.set_cf_all(false);
        let total = c.list.len();
        let title = if total > 1 { tf!("Name clash {} of {}", c.idx + 1, total) } else { ts("This name is already taken").to_string() };
        let msg = if kind == OpKind::Move {
            tf!("Moving into \"{}\" — an item with the same name is already there.", where_)
        } else {
            tf!("Copying into \"{}\" — an item with the same name is already there.", where_)
        };
        self.open_dialog(Dialog::Conflict, &title, &msg, "", 0, "", false);
    }

    fn conflict_choose(&mut self, which: i32) {
        let all = self.g().get_cf_all();
        let Some(c) = self.conflict.as_mut() else { return };
        let choice = match which {
            1 => Conflict::Replace,
            2 => Conflict::Skip,
            _ => Conflict::KeepBoth,
        };
        if all {
            for (src, _) in &c.list[c.idx..] {
                c.decided.insert(src.clone(), choice);
            }
            c.idx = c.list.len();
        } else {
            let src = c.list[c.idx].0.clone();
            c.decided.insert(src, choice);
            c.idx += 1;
        }
        if c.idx < c.list.len() {
            self.show_conflict();
            return;
        }
        let Some(mut c) = self.conflict.take() else { return };
        self.dialog = Dialog::None;
        self.g().set_dialog(0);
        self.ui.invoke_focus_main();
        c.op.policies = Some(c.decided);
        self.run_pending(c.op);
    }

    fn on_op_progress(&mut self, f: f32, t: String) {
        if !self.op_running {
            return;
        }
        let g = self.g();
        g.set_op_progress(f);
        g.set_op_text(ss(t));
    }

    fn on_op_done(&mut self, kind: OpKind, srcs: Vec<PathBuf>, dest: Option<PathBuf>, res: fsx::OpResult, hint: UndoHint) {
        self.op_running = false;
        let g = self.g();
        g.set_op_progress(-1.0);
        g.set_op_text(SharedString::default());
        g.set_op_cancelling(false);
        if res.cancelled {
            self.toast(if res.done > 0 { tf!("Cancelled — {} item{} done before that", res.done, if res.done == 1 { "" } else { "s" }) } else { ts("Cancelled").to_string() });
        } else if res.errors.is_empty() {
            if res.done > 0 {
                let what = if res.done == 1 && srcs.len() == 1 { quoted(&srcs.first().map(|p| fsx::file_name(p)).unwrap_or_default()) } else { tf!("{} items", res.done) };
                let mut msg = match kind {
                    OpKind::Trash => tf!("Moved {} to the {}", what, fsx::trash_word()),
                    OpKind::Zip => tf!("Created {}", res.created.first().map(|p| fsx::file_name(p)).unwrap_or_default()),
                    OpKind::Extract => tf!("Extracted {} item{} into \"{}\"", res.done, if res.done == 1 { "" } else { "s" }, res.created.first().map(|p| fsx::file_name(p)).unwrap_or_default()),
                    OpKind::Copy => tf!("Copied {}", what),
                    OpKind::Move => tf!("Moved {}", what),
                    OpKind::Delete => tf!("Deleted {}", what),
                };
                if res.skipped > 0 {
                    msg.push_str(&tf!(" · skipped {}", res.skipped));
                }
                if res.replaced > 0 {
                    msg.push_str(&tf!(" · replaced {} (old version{} in the {})", res.replaced, if res.replaced == 1 { " is" } else { "s are" }, fsx::trash_word()));
                }
                self.toast(msg);
            } else if res.skipped > 0 {
                self.toast(tf!("Skipped {} item{} — nothing was changed", res.skipped, if res.skipped == 1 { "" } else { "s" }));
            }
        } else {
            let first = res.errors[0].clone();
            let more = if res.errors.len() > 1 { tf!(" (+{} more)", res.errors.len() - 1) } else { String::new() };
            self.toast(tf!("Problem: {}{}", first, more));
        }
        if kind == OpKind::Move {
            if let Some((_, true)) = &self.clip {
                self.clip = None;
            }
            for (from, to) in &res.pairs {
                self.retag(from, to);
            }
        }
        // remember how to undo it (not when old items were replaced: those are in the bin)
        let act = match (&hint, kind) {
            (UndoHint::None, _) => None,
            _ if res.replaced > 0 || res.merged => None,
            (UndoHint::FolderFromSel(folder), _) => Some(UndoAct::MovedThenCreated(res.pairs.clone(), folder.clone())),
            (_, OpKind::Copy) | (_, OpKind::Zip) | (_, OpKind::Extract) if !res.created.is_empty() => Some(UndoAct::Created(res.created.clone())),
            (_, OpKind::Move) if !res.pairs.is_empty() => Some(UndoAct::Moved(res.pairs.clone())),
            (_, OpKind::Trash) if res.errors.is_empty() => Some(UndoAct::Trashed(srcs.clone())),
            _ => None,
        };
        if let Some(a) = act {
            self.undo.push(a);
            if self.undo.len() > 50 {
                self.undo.remove(0);
            }
        }
        if kind == OpKind::Extract || kind == OpKind::Zip {
            if let Some(c) = res.created.first() {
                if let Some(parent) = c.parent() {
                    if self.pane().loc.dir() == Some(parent) {
                        self.pane_mut().pending_select = vec![c.clone()];
                    }
                }
            }
        }
        // refresh affected folders right away (the watcher would also catch it)
        let mut dirs: HashSet<PathBuf> = srcs.iter().filter_map(|p| p.parent().map(|x| x.to_path_buf())).collect();
        if let Some(d) = &dest {
            dirs.insert(d.clone());
            if let Some(parent) = d.parent() {
                dirs.insert(parent.to_path_buf());
            }
            // select what we just created in the active pane
            if self.pane().loc.dir() == Some(d.as_path()) && !res.created.is_empty() {
                let created = res.created.clone();
                self.pane_mut().pending_select = created;
            }
        }
        self.on_fs_changed(dirs.into_iter().collect());
        if kind == OpKind::Trash || res.replaced > 0 {
            self.refresh_bin_views();
        }
        self.sync_commands();
        self.pump_queue();
    }

    fn targets(&self) -> Vec<PathBuf> {
        self.pane().selected_paths()
    }

    fn current_dir(&self) -> Option<PathBuf> {
        self.pane().loc.dir().map(|d| d.to_path_buf())
    }

    fn do_paste(&mut self, into: Option<PathBuf>) {
        let Some(dest) = into.or_else(|| self.current_dir()) else {
            self.toast(ts("Open a folder to paste into"));
            return;
        };
        let Some((srcs, cut)) = self.take_paste_source() else { return };
        self.start_op(if cut { OpKind::Move } else { OpKind::Copy }, srcs, Some(dest));
    }

    fn do_delete(&mut self, items: Vec<PathBuf>, permanent: bool) {
        if items.is_empty() {
            return;
        }
        let what = if items.len() == 1 { quoted(&fsx::file_name(&items[0])) } else { tf!("these {} items", items.len()) };
        if permanent {
            self.open_dialog(Dialog::ConfirmDelete { items }, ts("Delete permanently?"), &tf!("{} will be deleted for good. This can't be undone.", capitalize(&what)), "", 0, ts("Delete"), true);
        } else if self.s.confirm_delete {
            self.open_dialog(Dialog::ConfirmTrash { items }, &tf!("Move to the {}?", fsx::trash_word()), &tf!("{} will be moved to the {}.", capitalize(&what), fsx::trash_word()), "", 0, ts("Move"), true);
        } else {
            self.start_op(OpKind::Trash, items, None);
        }
    }

    // ───────────── dialogs ─────────────

    #[allow(clippy::too_many_arguments)]
    fn open_dialog(&mut self, d: Dialog, title: &str, message: &str, text: &str, sel_end: usize, ok: &str, danger: bool) {
        let (title, message, ok) = (&i18n::t(title), &i18n::t(message), &i18n::t(ok));
        let kind = match d {
            Dialog::FolderFromSel { .. } => 1,
            Dialog::ConfirmTrash { .. } | Dialog::ConfirmDelete { .. } | Dialog::ConfirmPurge { .. } | Dialog::ConfirmEmpty => 2,
            Dialog::Conflict => 7,
            Dialog::Connect => 8,
            Dialog::Settings => 3,
            Dialog::Props { .. } => 4,
            Dialog::Hashes => 5,
            Dialog::Batch => 6,
            Dialog::None => 0,
        };
        self.dialog = d;
        let g = self.g();
        g.set_dialog_title(ss(title));
        g.set_dialog_message(ss(message));
        g.set_dialog_text(ss(text));
        g.set_dialog_sel_end(sel_end as i32);
        g.set_dialog_ok_text(ss(ok));
        g.set_dialog_danger(danger);
        g.set_dialog_aux(SharedString::default());
        g.set_dialog(kind);
    }

    fn close_dialog(&mut self) {
        if matches!(self.dialog, Dialog::Conflict) && self.conflict.take().is_some() {
            self.toast(ts("Cancelled — nothing was copied or moved"));
            slint::Timer::single_shot(Duration::from_millis(10), || with_app(|a| a.pump_queue()));
        }
        self.dialog = Dialog::None;
        self.props_gen.fetch_add(1, Ordering::SeqCst);
        self.hash_gen.fetch_add(1, Ordering::SeqCst);
        self.g().set_dialog(0);
        self.ui.invoke_focus_main();
    }

    fn dialog_ok(&mut self, text: String) {
        let d = self.dialog.clone();
        let name = text.trim().to_string();
        match d {
            Dialog::None | Dialog::Settings | Dialog::Props { .. } => self.close_dialog(),
            Dialog::FolderFromSel { dir, items } => {
                if let Err(e) = fsx::valid_name(&name) {
                    self.toast(e);
                    return;
                }
                let target = dir.join(&name);
                if target.exists() {
                    self.toast(tf!("\"{}\" already exists here", name));
                    return;
                }
                if let Err(e) = std::fs::create_dir(&target) {
                    self.toast(tf!("Couldn't create it: {}", fsx::friendly_err(&e)));
                    return;
                }
                self.close_dialog();
                self.pane_mut().pending_select = vec![target.clone()];
                self.start_op_ex(OpKind::Move, items, Some(target.clone()), UndoHint::FolderFromSel(target));
            }
            Dialog::ConfirmTrash { items } => {
                self.close_dialog();
                self.start_op(OpKind::Trash, items, None);
            }
            Dialog::ConfirmDelete { items } => {
                self.close_dialog();
                self.start_op_ex(OpKind::Delete, items, None, UndoHint::None);
            }
            Dialog::Batch => self.batch_apply(),
            Dialog::Hashes => self.close_dialog(),
            Dialog::Conflict => {}
            Dialog::Connect => self.connect_ok(text),
            Dialog::ConfirmPurge { items } => {
                self.close_dialog();
                self.bin_purge_now(items);
            }
            Dialog::ConfirmEmpty => {
                self.close_dialog();
                self.bin_empty_now();
            }
        }
    }

    fn rename_selected(&mut self) {
        if self.pane().loc == Loc::Bin {
            return;
        }
        let sel = self.targets();
        if sel.len() > 1 {
            self.open_batch(sel);
            return;
        }
        if sel.len() != 1 {
            return;
        }
        let pi = self.ap();
        self.begin_rename(pi, sel[0].clone());
    }

    /// Rename right in the list (Explorer-style). The name without its extension is selected.
    fn begin_rename(&mut self, pi: usize, path: PathBuf) {
        let ti = self.cur;
        let idx = {
            let p = &self.tabs[ti].panes[pi];
            p.view.iter().position(|&i| p.entries[i].path == path)
        };
        let Some(idx) = idx else { return };
        let name = fsx::file_name(&path);
        let stem_len = fsx::split_name(&name, path.is_dir()).0.len();
        self.renaming = Some((pi, path));
        let g = self.g();
        g.set_rename_text(ss(&name));
        g.set_rename_sel_end(stem_len as i32);
        g.set_rename_pane(pi as i32);
        g.set_rename_index(idx as i32);
        self.ensure_visible(pi, idx as i32);
    }

    fn end_rename(&mut self) {
        if self.renaming.take().is_none() {
            return;
        }
        let g = self.g();
        g.set_rename_index(-1);
        let dirs = std::mem::take(&mut self.refresh_after_rename);
        if !dirs.is_empty() {
            self.on_fs_changed(dirs);
        }
        self.ui.invoke_focus_main();
    }

    /// `from_enter`: Enter was pressed (keep editing on a bad name); otherwise focus moved away.
    fn rename_commit(&mut self, text: String, from_enter: bool) {
        let Some((pi, path)) = self.renaming.clone() else { return };
        let name = text.trim().to_string();
        if name.is_empty() || name == fsx::file_name(&path) {
            self.end_rename();
            return;
        }
        let fail = |a: &mut App, msg: String| {
            a.toast(msg);
            if !from_enter {
                a.end_rename();
            }
        };
        if let Err(e) = fsx::valid_name(&name) {
            fail(self, e);
            return;
        }
        let Some(parent) = path.parent().map(|p| p.to_path_buf()) else { return };
        let target = parent.join(&name);
        if target.exists() && !same_name_ignoring_case(&target, &path) {
            fail(self, tf!("\"{}\" already exists here", name));
            return;
        }
        match std::fs::rename(&path, &target) {
            Ok(_) => {
                self.renaming = None;
                self.g().set_rename_index(-1);
                self.retag(&path, &target);
                self.undo.push(UndoAct::Renamed(vec![(path.clone(), target.clone())]));
                let ti = self.cur;
                self.tabs[ti].panes[pi].pending_select = vec![target];
                let mut dirs = std::mem::take(&mut self.refresh_after_rename);
                dirs.push(parent);
                self.on_fs_changed(dirs);
                self.sync_commands();
                self.ui.invoke_focus_main();
            }
            Err(e) => fail(self, tf!("Couldn't rename: {}", fsx::friendly_err(&e))),
        }
    }

    /// Like Explorer: the folder appears right away, ready to be named.
    fn new_folder(&mut self) {
        let Some(dir) = self.current_dir() else { return };
        let name = fsx::unique_new_name(&dir, ts("New folder"), true);
        let target = dir.join(&name);
        match std::fs::create_dir(&target) {
            Ok(_) => self.created_for_rename(dir, target),
            Err(e) => self.toast(tf!("Couldn't create a folder here: {}", fsx::friendly_err(&e))),
        }
    }

    fn new_file(&mut self) {
        let Some(dir) = self.current_dir() else { return };
        let name = fsx::unique_new_name(&dir, ts("New text document.txt"), false);
        let target = dir.join(&name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&target) {
            Ok(_) => self.created_for_rename(dir, target),
            Err(e) => self.toast(tf!("Couldn't create a file here: {}", fsx::friendly_err(&e))),
        }
    }

    fn created_for_rename(&mut self, dir: PathBuf, target: PathBuf) {
        self.undo.push(UndoAct::Created(vec![target.clone()]));
        self.pane_mut().pending_select = vec![target.clone()];
        self.rename_after_list = Some(target);
        self.on_fs_changed(vec![dir]);
        self.sync_commands();
    }

    fn folder_from_selection(&mut self) {
        let Some(dir) = self.current_dir() else { return };
        let items = self.targets();
        if items.is_empty() {
            return;
        }
        let name = fsx::unique_new_name(&dir, ts("New folder"), true);
        let n = name.len();
        let msg = tf!("{} item{} will be moved into it.", items.len(), if items.len() == 1 { "" } else { "s" });
        self.open_dialog(Dialog::FolderFromSel { dir, items }, ts("New folder with selection"), &msg, &name, n, ts("Create"), false);
    }

    fn show_properties(&mut self, items: Vec<PathBuf>) {
        let items = if items.is_empty() { self.current_dir().into_iter().collect() } else { items };
        if items.is_empty() {
            return;
        }
        let gen = self.props_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let mut rows: Vec<(String, String)> = Vec::new();
        let title;
        let single_file;
        if items.len() == 1 {
            let p = &items[0];
            title = fsx::file_name(p);
            let md = std::fs::metadata(p).ok();
            let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            single_file = !is_dir;
            let e = md.as_ref().map(|m| fsx::entry_from(p.clone(), title.clone(), m, String::new()));
            rows.push((ts("Type").into(), e.as_ref().map(fsx::type_name).unwrap_or_default()));
            rows.push((ts("Location").into(), p.parent().map(|x| x.display().to_string()).unwrap_or_else(|| p.display().to_string())));
            if is_dir {
                rows.push((ts("Size").into(), ts("Calculating…").into()));
                rows.push((ts("Contains").into(), ts("Calculating…").into()));
            } else if let Some(m) = &md {
                rows.push((ts("Size").into(), tf!("{} ({} bytes)", fsx::fmt_size(m.len()), fsx::fmt_count(m.len() as usize))));
            }
            if let Some(m) = &md {
                if let Ok(t) = m.created() {
                    rows.push((ts("Created").into(), fsx::fmt_time(Some(t))));
                }
                rows.push((ts("Modified").into(), fsx::fmt_time(m.modified().ok())));
                if let Ok(t) = m.accessed() {
                    rows.push((ts("Accessed").into(), fsx::fmt_time(Some(t))));
                }
                let mut attrs = Vec::new();
                if m.permissions().readonly() {
                    attrs.push(ts("Read-only"));
                }
                if e.as_ref().map(|x| x.hidden).unwrap_or(false) {
                    attrs.push(ts("Hidden"));
                }
                if !attrs.is_empty() {
                    rows.push((ts("Attributes").into(), attrs.join(", ")));
                }
            }
        } else {
            title = tf!("{} items", items.len());
            single_file = false;
            rows.push((ts("Location").into(), items[0].parent().map(|x| x.display().to_string()).unwrap_or_default()));
            rows.push((ts("Size").into(), ts("Calculating…").into()));
            rows.push((ts("Contains").into(), ts("Calculating…").into()));
        }
        self.props_rows = rows;
        self.push_props();
        self.open_dialog(Dialog::Props { items: items.clone() }, &title, "", "", 0, ts("Close"), false);
        if single_file {
            self.g().set_dialog_aux(ss("SHA-256"));
        }
        // background size calculation
        let needs_size = items.len() > 1 || items[0].is_dir();
        if needs_size {
            let cur = self.props_gen.clone();
            std::thread::spawn(move || {
                let (mut f, mut d, mut b) = (0u64, 0u64, 0u64);
                for it in &items {
                    match std::fs::symlink_metadata(it) {
                        Ok(m) if m.is_dir() => {
                            d += 1;
                            fsx::tree_stats(it, &mut f, &mut d, &mut b, &|| cur.load(Ordering::SeqCst) != gen);
                        }
                        Ok(m) => {
                            f += 1;
                            b += m.len();
                        }
                        Err(_) => {}
                    }
                }
                if items.len() == 1 {
                    d = d.saturating_sub(1);
                }
                let size = tf!("{} ({} bytes)", fsx::fmt_size(b), fsx::fmt_count(b as usize));
                let contains = tf!("{} files, {} folders", fsx::fmt_count(f as usize), fsx::fmt_count(d as usize));
                let _ = slint::invoke_from_event_loop(move || {
                    with_app(move |a| {
                        if a.props_gen.load(Ordering::SeqCst) == gen {
                            a.set_prop(ts("Size"), size);
                            a.set_prop(ts("Contains"), contains);
                        }
                    })
                });
            });
        }
    }

    fn set_prop(&mut self, key: &str, value: String) {
        if let Some(r) = self.props_rows.iter_mut().find(|(k, _)| k == key) {
            r.1 = value;
        } else {
            self.props_rows.push((key.into(), value));
        }
        self.push_props();
    }

    fn push_props(&self) {
        let rows: Vec<PropRow> = self.props_rows.iter().map(|(k, v)| PropRow { key: ss(k), value: ss(v) }).collect();
        self.g().set_dialog_props(ModelRc::new(VecModel::from(rows)));
    }

    fn dialog_aux(&mut self) {
        if let Dialog::Hashes = self.dialog {
            self.hash_copy_all();
            return;
        }
        if let Dialog::Props { items } = self.dialog.clone() {
            if items.len() == 1 {
                let p = items[0].clone();
                let gen = self.props_gen.load(Ordering::SeqCst);
                let cur = self.props_gen.clone();
                self.set_prop("SHA-256", ts("Calculating…").into());
                self.g().set_dialog_aux(SharedString::default());
                std::thread::spawn(move || {
                    let r = fsx::sha256_file(&p, &|| cur.load(Ordering::SeqCst) != gen);
                    let _ = slint::invoke_from_event_loop(move || {
                        with_app(move |a| {
                            if a.props_gen.load(Ordering::SeqCst) == gen {
                                a.set_prop("SHA-256", r.unwrap_or_else(|e| e));
                            }
                        })
                    });
                });
            }
        }
    }

    // ───────────── menus ─────────────

    fn show_menu(&mut self, entries: Vec<MenuEntry>, x: f32, y: f32) {
        self.menu_pos = (x, y);
        let h: f32 = entries.iter().map(|e| if e.separator { 9.0 } else if e.action.as_str() == "::tags" { 40.0 } else { 32.0 }).sum::<f32>() + 12.0;
        let g = self.g();
        g.set_menu(ModelRc::new(VecModel::from(entries)));
        g.set_menu_x(x);
        g.set_menu_y(y);
        g.set_menu_h(h);
        self.ui.invoke_show_menu();
    }

    fn row_right(&mut self, pi: usize, idx: usize, x: f32, y: f32) {
        self.activate_pane(pi);
        let ti = self.cur;
        let path = match self.tabs[ti].panes[pi].entry(idx) {
            Some(e) => e.path.clone(),
            None => return,
        };
        if !self.tabs[ti].panes[pi].selected.contains(&path) {
            let p = &mut self.tabs[ti].panes[pi];
            p.selected.clear();
            p.selected.insert(path);
            p.anchor = Some(idx);
            p.cursor = Some(idx);
            self.refresh_selection_flags(ti, pi);
            self.after_selection();
        }
        let targets = self.targets();
        let entries: Vec<Entry> = self.pane().selected_entries().into_iter().cloned().collect();
        self.menu = MenuCtx { pane: pi, targets: targets.clone(), side_path: None };
        if self.pane().loc == Loc::Bin {
            let mut del = mi("Delete permanently", 35, "purge", "Del");
            del.danger = true;
            let m = vec![
                mi(if entries.len() == 1 { ts("Restore") } else { ts("Restore these") }, 56, "restore", ""),
                mi("Copy original path", 41, "copy-original-path", ""),
                sep(),
                del,
            ];
            self.show_menu(m, x, y);
            return;
        }
        let single = entries.len() == 1;
        let all_dirs = entries.iter().all(|e| e.is_dir);
        let in_dir = self.current_dir().is_some();
        let mut m = Vec::new();
        m.push(mi("Open", 30, "open", "Enter"));
        if all_dirs {
            m.push(mi("Open in new tab", 42, "open-new-tab", ts("Middle-click")));
        }
        if single && !all_dirs && cfg!(windows) {
            m.push(mi("Open with…", 51, "open-with", ""));
        }
        if matches!(self.pane().loc, Loc::Search { .. } | Loc::Tag(_)) {
            m.push(mi("Open file location", 0, "open-location", ""));
        }
        if single && !all_dirs {
            m.push(mi("Quick look", 58, "quicklook", "Space"));
        }
        m.push(sep());
        m.push(mi("Cut", 31, "cut", "Ctrl+X"));
        m.push(mi("Copy", 32, "copy", "Ctrl+C"));
        if single && all_dirs && (self.clip.is_some() || self.system_clipboard_has_files()) {
            m.push(mi("Paste into folder", 33, "paste-into", ""));
        }
        m.push(mi("Copy path", 41, "copy-path", "Ctrl+Shift+C"));
        m.push(sep());
        if single {
            m.push(mi("Rename", 34, "rename", "F2"));
        } else {
            m.push(mi(&tf!("Rename {} items…", entries.len()), 59, "batch-rename", "F2"));
        }
        if in_dir {
            m.push(mi("New folder with selection", 47, "folder-from-sel", ""));
        }
        if single && all_dirs {
            let pinned = self.quick.iter().any(|(_, _, p)| p == &targets[0]);
            m.push(if pinned { mi("Unpin from sidebar", 40, "unpin", "") } else { mi("Pin to sidebar", 40, "pin", "") });
            m.push(mi("Open in terminal", 38, "terminal", ""));
            m.push(mi("Open in VS Code", 55, "vscode", ""));
        }
        m.push(sep());
        if entries.iter().any(|e| !e.is_dir) {
            m.push(mi("Hashes", 52, "hashes", "Ctrl+Shift+H"));
        }
        if single && !all_dirs && fsx::is_checksum_file(&entries[0].name) {
            m.push(mi("Verify checksums", 60, "verify-sums", ""));
        }
        if single && fsx::archive_kind(&entries[0].name).is_some() {
            let stem = fsx::archive_stem(&entries[0].name);
            m.push(mi(&tf!("Extract to \"{}\"", stem), 54, "extract-to", ""));
            m.push(mi("Extract here", 54, "extract-here", ""));
        }
        if in_dir {
            m.push(mi("Compress to ZIP", 53, "compress", ""));
        }
        m.push(tags_row());
        if cfg!(windows) {
            m.push(mi("Show more options", 61, "shell-menu", "Shift+F10"));
        }
        m.push(sep());
        let mut del = mi(&tf!("Move to {}", fsx::trash_word()), 35, "trash", "Del");
        del.danger = true;
        m.push(del);
        m.push(mi("Properties", 39, "properties", "Alt+Enter"));
        self.show_menu(m, x, y);
    }

    fn bg_menu(&mut self, pi: usize, x: f32, y: f32) {
        self.bg_down(pi, false);
        self.menu = MenuCtx { pane: pi, targets: vec![], side_path: None };
        if self.pane().loc == Loc::Bin {
            let mut empty = mi(&tf!("Empty {}", fsx::trash_word()), 35, "empty-bin", "");
            empty.danger = true;
            let has = !self.pane().entries.is_empty();
            let m = vec![
                mi("Restore all", 56, if has { "restore-all" } else { "" }, ""),
                mi("Select all", 45, "select-all", "Ctrl+A"),
                mi("Refresh", 43, "refresh", "F5"),
                sep(),
                empty,
            ];
            self.show_menu(m, x, y);
            return;
        }
        let in_dir = self.current_dir().is_some();
        let mut m = Vec::new();
        if in_dir {
            m.push(mi("New folder", 36, "new-folder", "Ctrl+Shift+N"));
            m.push(mi("New file", 37, "new-file", ""));
            m.push(sep());
            let mut p = mi("Paste", 33, "paste", "Ctrl+V");
            if self.clip.is_none() && !self.system_clipboard_has_files() {
                p.label = ss(ts("Paste (nothing copied)"));
                p.action = ss("");
            }
            m.push(p);
            m.push(sep());
        }
        if !self.undo.is_empty() {
            m.push(mi(&tf!("Undo {}", self.undo_label()), 56, "undo", "Ctrl+Z"));
        }
        m.push(mi("Select all", 45, "select-all", "Ctrl+A"));
        m.push(mi("Refresh", 43, "refresh", "F5"));
        if in_dir {
            m.push(mi("Open in terminal", 38, "terminal", ""));
            m.push(mi("Open in VS Code", 55, "vscode", ""));
            m.push(mi("Copy folder path", 41, "copy-folder-path", ""));
            if cfg!(windows) {
                m.push(mi("Show more options", 61, "shell-menu-bg", ""));
            }
            m.push(sep());
            m.push(mi("Properties", 39, "properties", "Alt+Enter"));
        }
        self.show_menu(m, x, y);
    }

    fn side_menu(&mut self, path: String, x: f32, y: f32) {
        if path == "::bin" {
            self.menu = MenuCtx { pane: self.ap(), targets: vec![], side_path: None };
            let mut empty = mi(&tf!("Empty {}", fsx::trash_word()), 35, "empty-bin", "");
            empty.danger = true;
            self.show_menu(vec![mi("Open", 30, "open-bin", ""), sep(), empty], x, y);
            return;
        }
        if let Some(i) = path.strip_prefix("::server:") {
            self.menu = MenuCtx { pane: self.ap(), targets: vec![], side_path: None };
            self.menu_tab = i.parse().unwrap_or(0);
            self.show_menu(vec![mi("Open", 30, "server-open", ""), mi("Open in new tab", 42, "server-open-tab", ""), sep(), mi("Remove from sidebar", 40, "server-remove", "")], x, y);
            return;
        }
        if let Some(f) = path.strip_prefix("::recentfile:") {
            let p = PathBuf::from(f);
            self.menu = MenuCtx { pane: self.ap(), targets: vec![p.clone()], side_path: Some(p.clone()) };
            let mut m = vec![mi("Open", 30, "recent-open", "")];
            if !fsx::file_name(&p).to_lowercase().ends_with(".lnk") {
                m.push(mi("Open file location", 0, "open-location", ""));
            }
            if self.s.recent_files.contains(&p) {
                m.push(sep());
                m.push(mi("Remove from list", 35, "recent-remove", ""));
            }
            self.show_menu(m, x, y);
            return;
        }
        if path.starts_with("::") {
            return;
        }
        let p = PathBuf::from(&path);
        self.menu = MenuCtx { pane: self.ap(), targets: vec![p.clone()], side_path: Some(p.clone()) };
        let pinned = self.quick.iter().any(|(_, _, q)| q == &p);
        let m = vec![
            mi("Open", 30, "side-open", ""),
            mi("Open in new tab", 42, "side-open-new-tab", ts("Middle-click")),
            sep(),
            if pinned { mi("Unpin from sidebar", 40, "unpin", "") } else { mi("Pin to sidebar", 40, "pin", "") },
            mi("Open in terminal", 38, "terminal", ""),
            mi("Copy path", 41, "copy-path", ""),
            sep(),
            mi("Properties", 39, "properties", ""),
        ];
        self.show_menu(m, x, y);
    }

    fn cmd_at(&mut self, name: &str, x: f32, y: f32) {
        self.menu = MenuCtx { pane: self.ap(), targets: self.targets(), side_path: None };
        let m = match name {
            "new-menu" => vec![mi("Folder", 36, "new-folder", "Ctrl+Shift+N"), mi("Text file / any file", 37, "new-file", ""), sep(), mi("Folder with selection", 47, "folder-from-sel", "")],
            "sort-menu" => {
                let k = self.s.sort_key;
                let mut v = vec![chk("Name", "sort:0", k == 0), chk("Date modified", "sort:1", k == 1), chk("Date created", "sort:5", k == 5), chk("Type", "sort:2", k == 2), chk("Size", "sort:3", k == 3)];
                if matches!(self.pane().loc, Loc::Search { .. }) {
                    v.push(chk("Location", "sort:4", k == 4));
                }
                v.push(sep());
                v.push(chk("Ascending", "asc", self.s.sort_asc));
                v.push(chk("Descending", "desc", !self.s.sort_asc));
                v
            }
            "tools-menu" => {
                let sel = self.pane().selected_entries().into_iter().cloned().collect::<Vec<_>>();
                let mut v = vec![
                    mi("Hashes", 52, "hashes", "Ctrl+Shift+H"),
                    mi("Verify a checksum file", 60, "verify-sums", ""),
                    sep(),
                    mi("Compress to ZIP", 53, "compress", ""),
                ];
                if sel.len() == 1 && fsx::archive_kind(&sel[0].name).is_some() {
                    v.push(mi(&tf!("Extract to \"{}\"", fsx::archive_stem(&sel[0].name)), 54, "extract-to", ""));
                    v.push(mi("Extract here", 54, "extract-here", ""));
                }
                v.push(mi("Batch rename", 59, "batch-rename", ""));
                v.push(sep());
                v.push(mi("Quick look", 58, "quicklook", "Space"));
                v.push(mi("Open in terminal", 38, "terminal", ""));
                v.push(mi("Open in VS Code", 55, "vscode", ""));
                v
            }
            "view-menu" => vec![
                chk("Details", "view:0", self.s.view_mode == 0),
                chk("Grid", "view:1", self.s.view_mode == 1),
                chk("Columns", "view:2", self.s.view_mode == 2),
                sep(),
                chk("Small icons", "grid:0", self.s.grid_size == 0),
                chk("Medium icons", "grid:1", self.s.grid_size == 1),
                chk("Large icons", "grid:2", self.s.grid_size == 2),
                sep(),
                chk("Friendly dates", "toggle-friendly", self.s.friendly_dates),
                chk("Preview pane", "toggle-preview", self.s.show_preview),
                chk("Hidden files", "toggle-hidden", self.s.show_hidden),
                chk("File extensions", "toggle-ext", self.s.show_ext),
                chk("Dual pane", "toggle-dual", self.tabs[self.cur].dual),
            ],
            _ => vec![
                mi(&if self.undo.is_empty() { ts("Undo").to_string() } else { tf!("Undo {}", self.undo_label()) }, 56, if self.undo.is_empty() { "" } else { "undo" }, "Ctrl+Z"),
                sep(),
                mi("Select all", 45, "select-all", "Ctrl+A"),
                mi("Invert selection", 45, "invert-sel", ""),
                sep(),
                mi("Open in terminal", 38, "terminal", ""),
                mi("Copy folder path", 41, "copy-folder-path", ""),
                mi("Properties", 39, "properties", "Alt+Enter"),
                sep(),
                mi("New window", 42, "new-window", "Ctrl+N"),
                mi("Command palette", 62, "palette", "Ctrl+Shift+P"),
                mi("Connect to server…", 28, "connect", ""),
                mi("Settings", 46, "settings", "Ctrl+,"),
            ],
        };
        self.show_menu(m, x, y);
    }

    /// Commands from the toolbar (`cmd`) and from menus (`menu-action`) end up here.
    pub fn action(&mut self, a: &str, from_menu: bool) {
        let targets = if from_menu { self.menu.targets.clone() } else { self.targets() };
        let pi = if from_menu { self.menu.pane } else { self.ap() };
        match a {
            "" => {}
            "open" => {
                if from_menu && targets.len() == 1 && targets[0].is_dir() {
                    self.navigate(pi, Loc::Dir(targets[0].clone()), true);
                } else {
                    self.open_selected();
                }
            }
            "open-new-tab" => {
                for t in targets.iter().filter(|t| t.is_dir()) {
                    self.open_tab(Loc::Dir(t.clone()), false);
                }
            }
            "open-with" => {
                if let Some(t) = targets.first() {
                    if let Err(e) = fsx::open_with(t) {
                        self.toast(e);
                    }
                }
            }
            "open-location" => {
                if let Some(t) = targets.first() {
                    if let Some(parent) = t.parent() {
                        let parent = parent.to_path_buf();
                        self.navigate_select(pi, parent, t.clone());
                    }
                }
            }
            "side-open" => {
                if let Some(p) = self.menu.side_path.clone() {
                    self.navigate(self.ap(), Loc::Dir(p), true);
                }
            }
            "side-open-new-tab" => {
                if let Some(p) = self.menu.side_path.clone() {
                    self.open_tab(Loc::Dir(p), true);
                }
            }
            "cut" => {
                if !targets.is_empty() {
                    self.set_clip(targets, true);
                }
            }
            "copy" => {
                if !targets.is_empty() {
                    self.set_clip(targets, false);
                }
            }
            "paste" => self.do_paste(None),
            "paste-into" => self.do_paste(targets.first().cloned()),
            "copy-path" => {
                let t = targets.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
                if !t.is_empty() {
                    self.copy_text(t);
                }
            }
            "copy-folder-path" => {
                if let Some(d) = self.current_dir() {
                    self.copy_text(d.display().to_string());
                }
            }
            "rename" => self.rename_selected(),
            "trash" | "delete" | "delete-perm" if self.tabs[self.cur].panes[pi].loc == Loc::Bin => self.bin_purge(targets),
            "trash" | "delete" => self.do_delete(targets, false),
            "delete-perm" => self.do_delete(targets, true),
            "new-folder" => self.new_folder(),
            "new-file" => self.new_file(),
            "folder-from-sel" => self.folder_from_selection(),
            "pin" => {
                if let Some(p) = targets.first().cloned() {
                    self.pin(p, true);
                }
            }
            "unpin" => {
                if let Some(p) = targets.first().cloned() {
                    self.pin(p, false);
                }
            }
            "terminal" => {
                let dir = targets.first().filter(|p| p.is_dir()).cloned().or_else(|| self.current_dir());
                if let Some(d) = dir {
                    if let Err(e) = fsx::open_terminal(&d) {
                        self.toast(e);
                    }
                }
            }
            "properties" => self.show_properties(targets),
            "select-all" => self.select_all(),
            "invert-sel" => self.invert_selection(),
            "refresh" => self.refresh(),
            "asc" | "desc" => {
                self.s.sort_asc = a == "asc";
                self.g().set_sort_asc(self.s.sort_asc);
                self.rebuild_all();
            }
            "toggle-preview" => {
                self.s.show_preview = !self.s.show_preview;
                self.g().set_show_preview(self.s.show_preview);
                self.update_preview();
            }
            "toggle-hidden" => {
                self.s.show_hidden = !self.s.show_hidden;
                self.g().set_show_hidden(self.s.show_hidden);
                self.rebuild_all();
                self.toast(if self.s.show_hidden { ts("Showing hidden files") } else { ts("Hiding hidden files") });
            }
            "toggle-ext" => {
                self.s.show_ext = !self.s.show_ext;
                self.g().set_set_show_ext(self.s.show_ext);
                self.rebuild_all();
            }
            "toggle-dual" => self.toggle_dual(),
            "toggle-friendly" => self.toggle_setting("friendly"),
            "hashes" => self.show_hashes(targets),
            "verify-sums" => {
                let t = targets.first().cloned().filter(|p| p.is_file() && fsx::is_checksum_file(&fsx::file_name(p)));
                match t {
                    Some(f) => self.verify_sums(f),
                    None => self.toast(ts("Select a .sha256 / .md5 / SHA256SUMS file first")),
                }
            }
            "compress" => self.compress(targets),
            "extract-here" | "extract-to" => {
                if let Some(t) = targets.first().cloned() {
                    self.extract(t, a == "extract-to");
                }
            }
            "batch-rename" => {
                if targets.len() > 1 {
                    self.open_batch(targets);
                } else {
                    self.toast(ts("Select several files to rename them together"));
                }
            }
            "vscode" => {
                let p = targets.first().cloned().or_else(|| self.current_dir());
                if let Some(p) = p {
                    if let Err(e) = fsx::open_vscode(&p) {
                        self.toast(e);
                    }
                }
            }
            "undo" => self.do_undo(),
            "palette" => self.palette_open(),
            "new-tab" => self.open_tab(Loc::Home, true),
            "close-tab" => {
                let c = self.cur;
                self.close_tab(c);
            }
            "reopen-tab" => match self.closed_tabs.pop() {
                Some(loc) => self.open_tab(loc, true),
                None => self.toast(ts("No closed tabs to reopen")),
            },
            "go-home" => self.navigate(pi, Loc::Home, true),
            "go-back" => self.go_back(),
            "go-forward" => self.go_forward(),
            "go-up" => self.go_up(),
            "focus-search" => self.ui.invoke_focus_search(),
            "edit-path" => self.g().set_edit_path(true),
            "connect" => self.open_connect(),
            "server-open" | "server-open-tab" => {
                if let Some(srv) = self.s.servers.get(self.menu_tab).cloned() {
                    self.open_server(srv, String::new(), String::new(), a == "server-open-tab");
                }
            }
            "server-remove" => {
                if self.menu_tab < self.s.servers.len() {
                    self.s.servers.remove(self.menu_tab);
                    self.s.save();
                    self.sync_sidebar();
                }
            }
            "recent-open" => {
                if let Some(p) = targets.first().cloned() {
                    self.open_path_str(&format!("::recentfile:{}", p.display()), false);
                }
            }
            "recent-remove" => {
                if let Some(p) = targets.first() {
                    self.s.recent_files.retain(|x| x != p);
                    self.s.save();
                    self.refresh_recent_files();
                }
            }
            "shell-menu" | "shell-menu-bg" => {
                #[cfg(windows)]
                self.shell_menu(a == "shell-menu-bg", false);
            }
            "tab-dup" => {
                if let Some(loc) = self.tab_loc(self.menu_tab) {
                    self.open_tab(loc, true);
                }
            }
            "tab-window" => {
                if let Some(loc) = self.tab_loc(self.menu_tab) {
                    self.spawn_window(&loc);
                    if self.tabs.len() > 1 {
                        let i = self.menu_tab;
                        self.close_tab(i);
                    }
                }
            }
            "tab-close" => {
                let i = self.menu_tab;
                self.close_tab(i);
            }
            "tab-close-others" => {
                let keep = self.menu_tab;
                for i in (0..self.tabs.len()).rev() {
                    if i != keep && self.tabs.len() > 1 {
                        self.close_tab(i);
                    }
                }
            }
            "tab-close-right" => {
                let keep = self.menu_tab;
                for i in (keep + 1..self.tabs.len()).rev() {
                    self.close_tab(i);
                }
            }
            "default-fm" => self.toggle_default_fm(),
            "settings-keys" => {
                self.g().set_set_tab(2);
                self.open_dialog(Dialog::Settings, ts("Settings"), "", "", 0, ts("Done"), false);
            }
            "settings-toolbar" => {
                self.g().set_set_tab(1);
                self.open_dialog(Dialog::Settings, ts("Settings"), "", "", 0, ts("Done"), false);
            }
            "shortcuts-reset" => {
                self.s.shortcuts.clear();
                self.s.save();
                self.sync_shortcut_rows();
                self.toast(ts("All shortcuts are back to normal"));
            }
            "restore" => self.bin_restore(targets),
            "restore-all" => {
                let all: Vec<PathBuf> = self.pane().entries.iter().map(|e| e.path.clone()).collect();
                self.bin_restore(all);
            }
            "purge" => self.bin_purge(targets),
            "empty-bin" => self.bin_empty(),
            "open-bin" => self.open_path_str("::bin", false),
            "cancel-op" => self.cancel_op(),
            "col-reset" => {
                self.s.col_w = crate::settings::DEFAULT_COL_W;
                self.sync_columns();
                self.s.save();
            }
            "copy-original-path" => {
                let t: Vec<String> = targets
                    .iter()
                    .filter_map(|p| self.pane().entries.iter().find(|e| &e.path == p).map(|e| PathBuf::from(&e.location).join(&e.name).display().to_string()))
                    .collect();
                if !t.is_empty() {
                    self.copy_text(t.join("\n"));
                }
            }
            "quicklook" => self.quicklook_toggle(),
            "settings" => {
                self.g().set_set_tab(0);
                self.open_dialog(Dialog::Settings, ts("Settings"), "", "", 0, ts("Done"), false);
            }
            "new-window" => {
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).spawn();
                }
            }
            _ => {
                if let Some(p) = a.strip_prefix("goto:") {
                    let p = p.to_string();
                    self.open_path_str(&p, false);
                } else if let Some(l) = a.strip_prefix("lang:") {
                    let l = l.to_string();
                    self.set_language(&l);
                } else if let Some(i) = a.strip_prefix("tab:") {
                    self.select_tab(i.parse().unwrap_or(0));
                } else if let Some(id) = a.strip_prefix("tb:") {
                    let id = id.to_string();
                    self.toggle_toolbar(&id);
                } else if let Some(c) = a.strip_prefix("col:") {
                    let i: usize = c.parse().unwrap_or(0).min(3);
                    self.s.col_show[i] = !self.s.col_show[i];
                    self.sync_columns();
                    self.s.save();
                } else if let Some(k) = a.strip_prefix("sort:") {
                    self.sort_by(k.parse().unwrap_or(0));
                } else if let Some(t) = a.strip_prefix("tag:") {
                    let i: u8 = t.parse().unwrap_or(0);
                    self.toggle_tag(i, targets);
                } else if let Some(g) = a.strip_prefix("grid:") {
                    self.s.grid_size = g.parse().unwrap_or(1);
                    self.g().set_grid_size(self.s.grid_size);
                    if self.s.view_mode != 1 {
                        self.s.view_mode = 1;
                        self.g().set_view_mode(1);
                        self.request_thumbs();
                    }
                } else if let Some(v) = a.strip_prefix("view:") {
                    self.s.view_mode = v.parse().unwrap_or(0);
                    self.g().set_view_mode(self.s.view_mode);
                    self.request_thumbs();
                    for pi in 0..2 {
                        self.child_dir[pi] = None;
                        self.refresh_columns(pi);
                        self.refresh_child_column(pi);
                    }
                    self.s.save();
                }
            }
        }
    }

    // ── Recycle Bin ──
    fn bin_items_for(&self, targets: &[PathBuf]) -> Vec<fsx::BinItem> {
        targets.iter().filter_map(|p| self.bin_items.get(p).cloned()).collect()
    }

    fn bin_restore(&mut self, targets: Vec<PathBuf>) {
        let items = self.bin_items_for(&targets);
        if items.is_empty() {
            return;
        }
        let dirs: Vec<PathBuf> = targets.iter().filter_map(|p| self.pane().entries.iter().find(|e| &e.path == p).map(|e| PathBuf::from(&e.location))).collect();
        self.toast(ts("Restoring…"));
        std::thread::spawn(move || {
            let (n, errs) = fsx::bin_restore(items);
            let msg = if errs.is_empty() {
                tf!("Restored {} item{}", n, if n == 1 { "" } else { "s" })
            } else if n > 0 {
                tf!("Restored {}, {} not: {}", n, errs.len(), errs[0])
            } else {
                tf!("Couldn't restore: {}", errs[0])
            };
            let _ = slint::invoke_from_event_loop(move || {
                with_app(move |a| {
                    a.toast(msg);
                    a.refresh_bin_views();
                    a.on_fs_changed(dirs);
                })
            });
        });
    }

    fn bin_purge(&mut self, targets: Vec<PathBuf>) {
        if targets.is_empty() {
            return;
        }
        let what = if targets.len() == 1 {
            quoted(&self.pane().entries.iter().find(|e| e.path == targets[0]).map(|e| e.name.clone()).unwrap_or_default())
        } else {
            tf!("these {} items", targets.len())
        };
        self.open_dialog(Dialog::ConfirmPurge { items: targets }, ts("Delete permanently?"), &tf!("{} will be deleted for good. This can't be undone.", capitalize(&what)), "", 0, ts("Delete"), true);
    }

    fn bin_purge_now(&mut self, targets: Vec<PathBuf>) {
        let items = self.bin_items_for(&targets);
        std::thread::spawn(move || {
            let msg = match fsx::bin_purge(items) {
                Ok(n) => tf!("Deleted {} item{} for good", n, if n == 1 { "" } else { "s" }),
                Err(e) => tf!("Couldn't delete: {}", e),
            };
            let _ = slint::invoke_from_event_loop(move || {
                with_app(move |a| {
                    a.toast(msg);
                    a.refresh_bin_views();
                })
            });
        });
    }

    fn bin_empty(&mut self) {
        let word = fsx::trash_word();
        self.open_dialog(Dialog::ConfirmEmpty, &tf!("Empty the {}?", word), &tf!("Everything in the {} will be deleted for good. This can't be undone.", word), "", 0, &tf!("Empty {}", word), true);
    }

    fn bin_empty_now(&mut self) {
        self.toast(tf!("Emptying the {}…", fsx::trash_word()));
        std::thread::spawn(move || {
            let msg = match fsx::bin_empty() {
                Ok(0) => tf!("The {} was already empty", fsx::trash_word()),
                Ok(n) => tf!("Emptied the {} ({} item{})", fsx::trash_word(), n, if n == 1 { "" } else { "s" }),
                Err(e) => tf!("Couldn't empty it: {}", e),
            };
            let _ = slint::invoke_from_event_loop(move || {
                with_app(move |a| {
                    a.toast(msg);
                    a.refresh_bin_views();
                })
            });
        });
    }

    // ── columns ──
    fn header_menu(&mut self, x: f32, y: f32) {
        let c = self.s.col_show;
        let m = vec![
            chk("Date modified", "col:0", c[0]),
            chk("Date created", "col:1", c[1]),
            chk("Type", "col:2", c[2]),
            chk("Size", "col:3", c[3]),
            sep(),
            mi("Reset column widths", 43, "col-reset", ""),
        ];
        self.menu = MenuCtx { pane: self.ap(), targets: vec![], side_path: None };
        self.show_menu(m, x, y);
    }

    fn col_resized(&mut self) {
        let g = self.g();
        self.s.col_w = [g.get_col_w_date(), g.get_col_w_created(), g.get_col_w_type(), g.get_col_w_size(), g.get_col_w_loc()];
        self.s.save();
    }

    fn pin(&mut self, p: PathBuf, yes: bool) {
        if !self.s.pins_set {
            self.s.pins = self.quick.iter().map(|(_, _, p)| p.clone()).collect();
            self.s.pins_set = true;
        }
        if yes {
            if !self.s.pins.contains(&p) {
                self.s.pins.push(p.clone());
            }
            self.toast(tf!("Pinned \"{}\"", fsx::file_name(&p)));
        } else {
            self.s.pins.retain(|x| x != &p);
            self.toast(tf!("Unpinned \"{}\"", fsx::file_name(&p)));
        }
        self.load_quick();
        self.s.save();
    }

    fn sort_by(&mut self, k: i32) {
        if self.s.sort_key == k {
            self.s.sort_asc = !self.s.sort_asc;
        } else {
            self.s.sort_key = k;
            self.s.sort_asc = !(k == 1 || k == 3 || k == 5); // newest / biggest first
        }
        let g = self.g();
        g.set_sort_key(self.s.sort_key);
        g.set_sort_asc(self.s.sort_asc);
        self.rebuild_all();
    }

    fn toggle_dual(&mut self) {
        let ti = self.cur;
        let t = &mut self.tabs[ti];
        t.dual = !t.dual;
        if t.dual {
            // the second pane starts where the first one is
            if t.panes[1].loc == Loc::Home && t.panes[0].loc != Loc::Home {
                t.panes[1].loc = t.panes[0].loc.clone();
                if matches!(t.panes[1].loc, Loc::Search { .. } | Loc::Tag(_)) {
                    t.panes[1].loc = Loc::Home;
                }
                self.start_listing(ti, 1, true);
            }
        } else {
            t.active = 0;
        }
        self.update_watches();
        self.sync_all();
    }

    pub fn refresh(&mut self) {
        let ti = self.cur;
        let pi = self.ap();
        self.start_listing(ti, pi, false);
        if self.pane().loc == Loc::Home {
            self.sync_sidebar();
        }
    }

    fn search_changed(&mut self, text: String) {
        let ti = self.cur;
        let pi = self.ap();
        self.tabs[ti].panes[pi].filter = text;
        self.rebuild(ti, pi);
        self.sync_panes_flags();
        self.sync_status();
    }

    fn search_submitted(&mut self, text: String) {
        let q = text.trim().to_string();
        if q.is_empty() {
            return;
        }
        let root = match &self.pane().loc {
            Loc::Dir(d) => d.clone(),
            Loc::Search { root, .. } => root.clone(),
            Loc::Home | Loc::Tag(_) | Loc::Bin => match dirs::home_dir() {
                Some(h) => h,
                None => return,
            },
        };
        let pi = self.ap();
        self.navigate(pi, Loc::Search { root, query: q }, true);
        self.ui.invoke_focus_main();
    }

    // ───────────── keyboard ─────────────

    pub fn key(&mut self, text: &str, ctrl: bool, shift: bool, alt: bool) -> bool {
        use slint::platform::Key;
        let k = |key: Key| -> SharedString { key.into() };
        let t = SharedString::from(text);
        let lower = text.to_lowercase();
        let grid = self.s.view_mode == 1;
        let cols = if grid { self.g().get_grid_cols().max(1) as i64 } else { 1 };

        if self.g().get_dialog() != 0 {
            return false;
        }
        if self.g().get_pal() {
            return false;
        }
        // the user's own shortcuts win
        if let Some(combo) = combo_of(text, ctrl, shift, alt) {
            if self.renaming.is_none() {
                if let Some((a, _)) = self.s.shortcuts.iter().find(|(_, c)| *c == combo).cloned() {
                    self.action(&a, false);
                    return true;
                }
            }
        }
        if self.renaming.is_some() {
            // the name field has the keyboard; only Esc reaches us
            if t == k(Key::Escape) {
                self.end_rename();
                return true;
            }
            return false;
        }
        if self.g().get_ql() {
            if t == k(Key::Escape) || text == " " || t == k(Key::Return) {
                self.quicklook_close();
            } else if t == k(Key::LeftArrow) || t == k(Key::UpArrow) {
                self.move_cursor(-1, false, None);
                self.quicklook_load();
            } else if t == k(Key::RightArrow) || t == k(Key::DownArrow) {
                self.move_cursor(1, false, None);
                self.quicklook_load();
            }
            return true;
        }
        if ctrl && !alt {
            match lower.as_str() {
                "p" if shift => { self.palette_open(); return true; }
                "t" if shift => { self.action("reopen-tab", false); return true; }
                "t" => { self.open_tab(Loc::Home, true); return true; }
                "z" => { self.do_undo(); return true; }
                "h" if shift => { self.action("hashes", false); return true; }
                "w" => { let c = self.cur; self.close_tab(c); return true; }
                "n" if shift => { self.new_folder(); return true; }
                "n" => { self.action("new-window", false); return true; }
                "c" if shift => { self.action("copy-path", false); return true; }
                "c" => { self.action("copy", false); return true; }
                "x" => { self.action("cut", false); return true; }
                "v" => { self.action("paste", false); return true; }
                "a" => { self.select_all(); return true; }
                "l" => { self.g().set_edit_path(true); return true; }
                "f" => { self.ui.invoke_focus_search(); return true; }
                "h" => { self.action("toggle-hidden", false); return true; }
                "p" => { self.action("toggle-preview", false); return true; }
                "r" => { self.refresh(); return true; }
                "," => { self.action("settings", false); return true; }
                "\t" => {
                    let n = self.tabs.len();
                    let next = if shift { (self.cur + n - 1) % n } else { (self.cur + 1) % n };
                    self.select_tab(next);
                    return true;
                }
                d if d.len() == 1 && d.chars().next().map(|c| ('1'..='9').contains(&c)).unwrap_or(false) => {
                    let n: usize = d.parse().unwrap_or(1);
                    let idx = if n == 9 { self.tabs.len() - 1 } else { n - 1 };
                    self.select_tab(idx);
                    return true;
                }
                _ => {}
            }
        }
        if alt {
            if t == k(Key::LeftArrow) { self.go_back(); return true; }
            if t == k(Key::RightArrow) { self.go_forward(); return true; }
            if t == k(Key::UpArrow) { self.go_up(); return true; }
            if t == k(Key::Return) { self.action("properties", false); return true; }
            if lower == "d" { self.g().set_edit_path(true); return true; }
            return false;
        }
        if t == k(Key::F10) && shift {
            #[cfg(windows)]
            self.shell_menu(self.pane().selected.is_empty(), true);
            return true;
        }
        // column view: ← back to the parent column, → into the selected folder
        if self.s.view_mode == 2 && !shift {
            if t == k(Key::LeftArrow) {
                self.go_up();
                return true;
            }
            if t == k(Key::RightArrow) {
                let sel: Vec<Entry> = self.pane().selected_entries().into_iter().cloned().collect();
                if sel.len() == 1 && sel[0].is_dir {
                    let pi = self.ap();
                    self.select_first[pi] = true;
                    self.navigate(pi, Loc::Dir(sel[0].path.clone()), true);
                }
                return true;
            }
        }
        if t == k(Key::F5) { self.refresh(); return true; }
        if t == k(Key::F2) { self.rename_selected(); return true; }
        if t == k(Key::F3) { self.ui.invoke_focus_search(); return true; }
        if t == k(Key::F4) { self.g().set_edit_path(true); return true; }
        if t == k(Key::Delete) {
            if self.pane().loc == Loc::Bin { self.bin_purge(self.targets()); } else { self.do_delete(self.targets(), shift); }
            return true;
        }
        if t == k(Key::Backspace) { self.go_back(); return true; }
        if t == k(Key::Return) { self.open_selected(); return true; }
        if text == " " && !ctrl { self.quicklook_toggle(); return true; }
        if t == k(Key::Escape) {
            if !self.pane().filter.is_empty() {
                self.g().set_search_text(SharedString::default());
                self.search_changed(String::new());
            } else {
                self.clear_selection();
            }
            return true;
        }
        if t == k(Key::Tab) && !ctrl {
            if self.tabs[self.cur].dual {
                let other = 1 - self.ap();
                self.activate_pane(other);
                return true;
            }
            return false;
        }
        if t == k(Key::DownArrow) { self.move_cursor(cols, shift, None); return true; }
        if t == k(Key::UpArrow) { self.move_cursor(-cols, shift, None); return true; }
        if grid && t == k(Key::RightArrow) { self.move_cursor(1, shift, None); return true; }
        if grid && t == k(Key::LeftArrow) { self.move_cursor(-1, shift, None); return true; }
        if t == k(Key::PageDown) { self.move_cursor(15 * cols, shift, None); return true; }
        if t == k(Key::PageUp) { self.move_cursor(-15 * cols, shift, None); return true; }
        if t == k(Key::Home) { self.move_cursor(0, shift, Some(0)); return true; }
        if t == k(Key::End) {
            let n = self.pane().view.len();
            if n > 0 { self.move_cursor(0, shift, Some(n - 1)); }
            return true;
        }
        // type-ahead: jump to the first item starting with the typed letters
        if !ctrl && text.chars().count() == 1 {
            let c = text.chars().next().unwrap();
            if !c.is_control() && (c as u32) < 0xF700 {
                self.type_ahead(text);
                return true;
            }
        }
        false
    }

    // ───────────── settings ─────────────

    fn set_theme(&mut self, dark: bool) {
        self.s.dark = dark;
        self.apply_theme();
        self.apply_titlebar();
        self.s.save();
    }

    fn set_accent(&mut self, i: usize) {
        self.s.accent = i.min(ACCENTS.len() - 1);
        self.g().set_accent_index(self.s.accent as i32);
        self.apply_theme();
        self.s.save();
    }

    fn toggle_setting(&mut self, which: &str) {
        match which {
            "hidden" => self.action("toggle-hidden", false),
            "ext" => self.action("toggle-ext", false),
            "restore" => {
                self.s.restore = !self.s.restore;
                self.g().set_set_restore(self.s.restore);
            }
            "confirm" => {
                self.s.confirm_delete = !self.s.confirm_delete;
                self.g().set_set_confirm_delete(self.s.confirm_delete);
            }
            "friendly" => {
                self.s.friendly_dates = !self.s.friendly_dates;
                self.g().set_friendly_dates(self.s.friendly_dates);
                self.rebuild_all();
            }
            _ => {}
        }
        self.s.save();
    }

    pub fn save_session(&mut self) {
        self.s.tabs = self
            .tabs
            .iter()
            .map(|t| match &t.panes[t.active].loc {
                Loc::Home => "::home".to_string(),
                Loc::Dir(d) => d.display().to_string(),
                Loc::Search { root, .. } => root.display().to_string(),
                Loc::Tag(_) => "::home".to_string(),
                Loc::Bin => "::home".to_string(),
            })
            .collect();
        self.s.tags = self.tags.iter().filter(|(_, m)| **m != 0).map(|(p, m)| (p.clone(), *m)).collect();
        self.s.active_tab = self.cur;
        self.s.save();
    }
}

// ───────────────────────── v0.2 features ─────────────────────────

impl App {
    pub fn apply_titlebar(&self) {
        use slint::winit_030::{winit, WinitWindowAccessor};
        let dark = self.s.dark;
        self.ui.window().with_winit_window(|w| {
            w.set_theme(Some(if dark { winit::window::Theme::Dark } else { winit::window::Theme::Light }));
        });
    }

    // ── tags ──
    fn retag(&mut self, from: &Path, to: &Path) {
        if let Some(m) = self.tags.remove(from) {
            self.tags.insert(to.to_path_buf(), m);
        }
    }

    fn toggle_tag(&mut self, i: u8, targets: Vec<PathBuf>) {
        if targets.is_empty() || i > 5 {
            return;
        }
        let bit = 1u8 << i;
        let all = targets.iter().all(|p| self.tags.get(p).map(|m| m & bit != 0).unwrap_or(false));
        for p in &targets {
            let m = self.tags.entry(p.clone()).or_insert(0);
            if all { *m &= !bit } else { *m |= bit }
        }
        self.tags.retain(|_, m| *m != 0);
        self.s.tags = self.tags.iter().map(|(p, m)| (p.clone(), *m)).collect();
        self.s.save();
        self.toast(if all { tf!("Removed the {} tag", ts(TAG_NAMES[i as usize])) } else { tf!("Tagged {} item{} {}", targets.len(), if targets.len() == 1 { "" } else { "s" }, ts(TAG_NAMES[i as usize])) });
        // tag views need a fresh list, folder views just new dots
        for ti in 0..self.tabs.len() {
            for pi in 0..2 {
                if matches!(self.tabs[ti].panes[pi].loc, Loc::Tag(_)) {
                    self.start_listing(ti, pi, false);
                } else {
                    self.rebuild(ti, pi);
                }
            }
        }
        self.sync_sidebar();
    }

    // ── undo ──
    fn undo_label(&self) -> String {
        match self.undo.last() {
            Some(UndoAct::Renamed(v)) if v.len() == 1 => tf!("rename of \"{}\"", fsx::file_name(&v[0].1)),
            Some(UndoAct::Renamed(v)) => tf!("rename of {} items", v.len()),
            Some(UndoAct::Created(v)) if v.len() == 1 => tf!("create \"{}\"", fsx::file_name(&v[0])),
            Some(UndoAct::Created(v)) => tf!("copy of {} items", v.len()),
            Some(UndoAct::Moved(v)) => tf!("move of {} item{}", v.len(), if v.len() == 1 { "" } else { "s" }),
            Some(UndoAct::MovedThenCreated(_, f)) => tf!("folder \"{}\"", fsx::file_name(f)),
            Some(UndoAct::Trashed(v)) => tf!("delete of {} item{}", v.len(), if v.len() == 1 { "" } else { "s" }),
            None => String::new(),
        }
    }

    fn do_undo(&mut self) {
        let label = self.undo_label();
        let Some(act) = self.undo.pop() else {
            self.toast(ts("Nothing to undo"));
            return;
        };
        match act {
            UndoAct::Renamed(pairs) => {
                let mut dirs = Vec::new();
                let mut errs = 0;
                for (from, to) in pairs.iter().rev() {
                    if std::fs::rename(to, from).is_ok() {
                        self.retag(to, from);
                        if let Some(d) = from.parent() {
                            dirs.push(d.to_path_buf());
                        }
                    } else {
                        errs += 1;
                    }
                }
                self.pane_mut().pending_select = pairs.iter().map(|(f, _)| f.clone()).collect();
                self.on_fs_changed(dirs);
                self.toast(if errs == 0 { tf!("Undid {}", label) } else { tf!("Undid part of it ({} failed)", errs) });
            }
            UndoAct::Created(paths) => {
                self.start_op_ex(OpKind::Trash, paths, None, UndoHint::None);
                self.toast(tf!("Undid {}", label));
            }
            UndoAct::Moved(pairs) => self.spawn_undo(label, pairs, None),
            UndoAct::MovedThenCreated(pairs, folder) => self.spawn_undo(label, pairs, Some(folder)),
            UndoAct::Trashed(paths) => {
                std::thread::spawn(move || {
                    let r = fsx::restore_from_trash(&paths);
                    let dirs: Vec<PathBuf> = paths.iter().filter_map(|p| p.parent().map(|x| x.to_path_buf())).collect();
                    let msg = match r {
                        Ok(n) => tf!("Restored {} item{}", n, if n == 1 { "" } else { "s" }),
                        Err(e) => tf!("Couldn't undo: {}", e),
                    };
                    let _ = slint::invoke_from_event_loop(move || with_app(move |a| a.on_undo_done(msg, dirs, paths)));
                });
            }
        }
        self.sync_commands();
    }

    fn spawn_undo(&mut self, label: String, pairs: Vec<(PathBuf, PathBuf)>, folder: Option<PathBuf>) {
        std::thread::spawn(move || {
            let res = fsx::move_back(&pairs);
            if let Some(f) = &folder {
                if std::fs::read_dir(f).map(|mut d| d.next().is_none()).unwrap_or(false) {
                    let _ = std::fs::remove_dir(f);
                }
            }
            let mut dirs: Vec<PathBuf> = pairs.iter().flat_map(|(a, b)| [a.parent(), b.parent()]).flatten().map(|x| x.to_path_buf()).collect();
            if let Some(f) = &folder {
                if let Some(p) = f.parent() {
                    dirs.push(p.to_path_buf());
                }
            }
            let msg = if res.errors.is_empty() { tf!("Undid {}", label) } else { tf!("Undo problem: {}", res.errors[0]) };
            let back: Vec<PathBuf> = pairs.iter().map(|(a, _)| a.clone()).collect();
            let moved: Vec<(PathBuf, PathBuf)> = pairs.clone();
            let _ = slint::invoke_from_event_loop(move || {
                with_app(move |a| {
                    for (from, to) in &moved {
                        a.retag(to, from);
                    }
                    a.on_undo_done(msg, dirs, back)
                })
            });
        });
    }

    fn on_undo_done(&mut self, msg: String, dirs: Vec<PathBuf>, select: Vec<PathBuf>) {
        self.pane_mut().pending_select = select;
        self.on_fs_changed(dirs);
        self.toast(msg);
        self.sync_commands();
    }

    // ── hashes ──
    fn show_hashes(&mut self, targets: Vec<PathBuf>) {
        let files: Vec<PathBuf> = targets.into_iter().filter(|p| p.is_file()).collect();
        if files.is_empty() {
            self.toast(ts("Select one or more files to hash"));
            return;
        }
        let gen = self.hash_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let cur = self.hash_gen.clone();
        self.hash_multi = files.len() > 1;
        let g = self.g();
        g.set_hash_verify(false);
        g.set_hash_compare(SharedString::default());
        g.set_hash_match(SharedString::default());
        g.set_hash_match_state(0);
        g.set_hash_progress(0.0);
        g.set_hash_busy(true);
        if files.len() == 1 {
            let f = files[0].clone();
            let size = std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
            g.set_hash_file(ss(format!("{} · {}", fsx::file_name(&f), fsx::fmt_size(size))));
            self.hash_rows = fsx::HASH_ALGOS.iter().map(|a| (a.to_string(), ts("Calculating…").to_string())).collect();
            self.push_hash_rows();
            self.open_dialog(Dialog::Hashes, ts("Hashes"), "", "", 0, ts("Done"), false);
            std::thread::spawn(move || {
                let c2 = cur.clone();
                let prog = move |f: f32| {
                    let _ = slint::invoke_from_event_loop(move || with_app(move |a| a.on_hash_progress(gen, f)));
                };
                let r = fsx::hash_file(&f, &fsx::HASH_ALGOS, &prog, &move || c2.load(Ordering::SeqCst) != gen);
                let rows = r.unwrap_or_else(|e| vec![(ts("Error").into(), e)]);
                let _ = slint::invoke_from_event_loop(move || with_app(move |a| a.on_hash_done(gen, rows)));
            });
        } else {
            g.set_hash_file(ss(tf!("{} files · SHA-256", files.len())));
            self.hash_rows = files.iter().map(|p| (fsx::file_name(p), ts("Calculating…").to_string())).collect();
            self.push_hash_rows();
            self.open_dialog(Dialog::Hashes, ts("Hashes"), "", "", 0, ts("Done"), false);
            std::thread::spawn(move || {
                let n = files.len();
                for (i, f) in files.iter().enumerate() {
                    let c2 = cur.clone();
                    if c2.load(Ordering::SeqCst) != gen {
                        return;
                    }
                    let r = fsx::hash_file(f, &["SHA-256"], &|_| {}, &move || c2.load(Ordering::SeqCst) != gen);
                    let v = match r {
                        Ok(v) => v.into_iter().next().map(|x| x.1).unwrap_or_default(),
                        Err(e) => e,
                    };
                    let prog = (i + 1) as f32 / n as f32;
                    let _ = slint::invoke_from_event_loop(move || {
                        with_app(move |a| {
                            if a.hash_gen.load(Ordering::SeqCst) == gen {
                                if let Some(r) = a.hash_rows.get_mut(i) {
                                    r.1 = v;
                                }
                                a.push_hash_rows();
                                a.on_hash_progress(gen, prog);
                            }
                        })
                    });
                }
                let _ = slint::invoke_from_event_loop(move || with_app(move |a| a.on_hash_done(gen, Vec::new())));
            });
        }
    }

    fn push_hash_rows(&self) {
        let rows: Vec<PropRow> = self.hash_rows.iter().map(|(k, v)| PropRow { key: ss(k), value: ss(v) }).collect();
        self.g().set_hash_rows(ModelRc::new(VecModel::from(rows)));
    }

    fn on_hash_progress(&mut self, gen: u64, f: f32) {
        if self.hash_gen.load(Ordering::SeqCst) == gen {
            self.g().set_hash_progress(f);
        }
    }

    fn on_hash_done(&mut self, gen: u64, rows: Vec<(String, String)>) {
        if self.hash_gen.load(Ordering::SeqCst) != gen {
            return;
        }
        if !rows.is_empty() {
            self.hash_rows = rows;
            self.push_hash_rows();
        }
        let g = self.g();
        g.set_hash_busy(false);
        g.set_hash_progress(1.0);
        if self.hash_multi {
            g.set_dialog_aux(ss(ts("Copy all")));
            if self.hash_rows.len() == 2 {
                let same = self.hash_rows[0].1 == self.hash_rows[1].1;
                g.set_hash_match(ss(if same { ts("These two files are identical") } else { ts("These two files are different") }));
                g.set_hash_match_state(if same { 1 } else { 2 });
            }
        } else {
            g.set_dialog_aux(ss(ts("Copy all")));
        }
        let cmp = g.get_hash_compare().to_string();
        if !cmp.is_empty() {
            self.hash_compare(cmp);
        }
    }

    fn hash_compare(&mut self, text: String) {
        let t: String = text.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase();
        let g = self.g();
        if t.is_empty() {
            g.set_hash_match(SharedString::default());
            g.set_hash_match_state(0);
            return;
        }
        if let Some((name, _)) = self.hash_rows.iter().find(|(_, v)| *v == t) {
            g.set_hash_match(ss(tf!("Match — same as {}", name)));
            g.set_hash_match_state(1);
        } else if let Some(algo) = fsx::algo_for_hash(&t) {
            if self.hash_multi {
                g.set_hash_match(ss(ts("No file here has that hash")));
            } else {
                g.set_hash_match(ss(tf!("Doesn't match — {} is different", algo)));
            }
            g.set_hash_match_state(2);
        } else {
            g.set_hash_match(ss(ts("That doesn't look like a CRC32, MD5, SHA-1, SHA-256 or SHA-512 hash")));
            g.set_hash_match_state(2);
        }
    }

    fn hash_copy(&mut self, which: String) {
        if let Some((_, v)) = self.hash_rows.iter().find(|(k, _)| *k == which) {
            let v = v.clone();
            self.copy_text_msg(v, ts("Hash copied"));
        }
    }

    fn hash_copy_all(&mut self) {
        let text = if self.hash_multi {
            self.hash_rows.iter().map(|(n, h)| format!("{}  {}", h, n)).collect::<Vec<_>>().join("\n")
        } else {
            self.hash_rows.iter().map(|(n, h)| format!("{}: {}", n, h)).collect::<Vec<_>>().join("\n")
        };
        self.copy_text_msg(text, ts("All hashes copied"));
    }

    fn copy_text_msg(&mut self, text: String, ok: &str) {
        if self.sys_clip.is_none() {
            self.sys_clip = arboard::Clipboard::new().ok();
        }
        match self.sys_clip.as_mut().map(|c| c.set_text(text)) {
            Some(Ok(_)) => self.toast(ok.to_string()),
            _ => self.toast(ts("Couldn't access the clipboard")),
        }
    }

    fn verify_sums(&mut self, file: PathBuf) {
        let Ok(text) = std::fs::read_to_string(&file) else {
            self.toast(ts("Couldn't read that checksum file"));
            return;
        };
        let list = fsx::parse_checksums(&text);
        if list.is_empty() {
            self.toast(ts("No checksums found in that file"));
            return;
        }
        let dir = file.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let gen = self.hash_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let cur = self.hash_gen.clone();
        self.hash_multi = true;
        self.hash_rows = list.iter().map(|(_, n)| (n.clone(), ts("Checking…").to_string())).collect();
        self.push_hash_rows();
        let g = self.g();
        g.set_hash_file(ss(tf!("{} · {} file{}", fsx::file_name(&file), list.len(), if list.len() == 1 { "" } else { "s" })));
        g.set_hash_verify(true);
        g.set_hash_compare(SharedString::default());
        g.set_hash_match(SharedString::default());
        g.set_hash_match_state(0);
        g.set_hash_busy(true);
        g.set_hash_progress(0.0);
        self.open_dialog(Dialog::Hashes, ts("Verify checksums"), "", "", 0, ts("Done"), false);
        std::thread::spawn(move || {
            let n = list.len();
            let mut bad = 0;
            for (i, (want, name)) in list.iter().enumerate() {
                if cur.load(Ordering::SeqCst) != gen {
                    return;
                }
                let path = dir.join(name);
                let status = if !path.is_file() {
                    bad += 1;
                    ts("✗ Missing").to_string()
                } else {
                    let algo = fsx::algo_for_hash(want).unwrap_or("SHA-256");
                    let c2 = cur.clone();
                    match fsx::hash_file(&path, &[algo], &|_| {}, &move || c2.load(Ordering::SeqCst) != gen) {
                        Ok(v) if v.first().map(|x| x.1 == *want).unwrap_or(false) => format!("✓ OK ({})", algo),
                        Ok(_) => {
                            bad += 1;
                            tf!("✗ Doesn't match ({})", algo)
                        }
                        Err(e) => {
                            bad += 1;
                            format!("✗ {}", e)
                        }
                    }
                };
                let prog = (i + 1) as f32 / n as f32;
                let _ = slint::invoke_from_event_loop(move || {
                    with_app(move |a| {
                        if a.hash_gen.load(Ordering::SeqCst) == gen {
                            if let Some(r) = a.hash_rows.get_mut(i) {
                                r.1 = status;
                            }
                            a.push_hash_rows();
                            a.on_hash_progress(gen, prog);
                        }
                    })
                });
            }
            let _ = slint::invoke_from_event_loop(move || {
                with_app(move |a| {
                    if a.hash_gen.load(Ordering::SeqCst) != gen {
                        return;
                    }
                    let g = a.g();
                    g.set_hash_busy(false);
                    g.set_hash_match(ss(if bad == 0 { tf!("All {} file{} OK", n, if n == 1 { "" } else { "s" }) } else { tf!("{} of {} failed", bad, n) }));
                    g.set_hash_match_state(if bad == 0 { 1 } else { 2 });
                })
            });
        });
    }

    // ── archives ──
    fn compress(&mut self, targets: Vec<PathBuf>) {
        let Some(dir) = self.current_dir() else {
            self.toast(ts("Open a folder first"));
            return;
        };
        if targets.is_empty() {
            self.toast(ts("Select what to compress"));
            return;
        }
        let base = if targets.len() == 1 { fsx::split_name(&fsx::file_name(&targets[0]), targets[0].is_dir()).0 } else { fsx::file_name(&dir) };
        let name = fsx::unique_new_name(&dir, &format!("{}.zip", if base.is_empty() { ts("Archive").into() } else { base }), false);
        self.start_op(OpKind::Zip, targets, Some(dir.join(name)));
    }

    fn extract(&mut self, archive: PathBuf, to_folder: bool) {
        let Some(dir) = archive.parent().map(|p| p.to_path_buf()) else { return };
        let name = fsx::file_name(&archive);
        if fsx::archive_kind(&name).is_none() {
            self.toast(ts("SKJ Files can extract .zip, .tar, .tar.gz and .gz for now"));
            return;
        }
        let dest = if to_folder { dir.join(fsx::unique_new_name(&dir, &fsx::archive_stem(&name), true)) } else { dir };
        self.start_op(OpKind::Extract, vec![archive], Some(dest));
    }

    // ── batch rename ──
    fn open_batch(&mut self, items: Vec<PathBuf>) {
        self.batch_items = items;
        let g = self.g();
        g.set_batch_find(SharedString::default());
        g.set_batch_replace(SharedString::default());
        g.set_batch_pattern(SharedString::default());
        let n = self.batch_items.len();
        self.open_dialog(Dialog::Batch, &tf!("Rename {} items", n), ts("Find & replace, and/or a pattern. Tokens: {name} {n} {nn} {nnn} {date} {ext}"), "", 0, ts("Rename"), false);
        self.batch_preview();
    }

    fn batch_plan(&self) -> Vec<(PathBuf, PathBuf)> {
        let g = self.g();
        let (find, rep, pat) = (g.get_batch_find().to_string(), g.get_batch_replace().to_string(), g.get_batch_pattern().to_string());
        let mut out = Vec::new();
        for (i, p) in self.batch_items.iter().enumerate() {
            let Ok(md) = std::fs::metadata(p) else { continue };
            let e = fsx::entry_from(p.clone(), fsx::file_name(p), &md, String::new());
            let new = fsx::batch_name(&e, &find, &rep, &pat, i + 1);
            if let Some(parent) = p.parent() {
                out.push((p.clone(), parent.join(new)));
            }
        }
        out
    }

    fn batch_preview(&mut self) {
        let plan = self.batch_plan();
        let rows: Vec<PropRow> = plan.iter().take(8).map(|(a, b)| PropRow { key: ss(fsx::file_name(a)), value: ss(fsx::file_name(b)) }).collect();
        let changed = plan.iter().filter(|(a, b)| a != b).count();
        let g = self.g();
        g.set_batch_preview(ModelRc::new(VecModel::from(rows)));
        g.set_batch_summary(ss(if plan.len() > 8 { tf!("{} of {} names change · showing the first 8", changed, plan.len()) } else { tf!("{} of {} names change", changed, plan.len()) }));
    }

    fn batch_apply(&mut self) {
        let plan: Vec<(PathBuf, PathBuf)> = self.batch_plan().into_iter().filter(|(a, b)| a != b).collect();
        if plan.is_empty() {
            self.close_dialog();
            return;
        }
        // refuse duplicates and invalid names up front
        let mut seen = HashSet::new();
        for (_, b) in &plan {
            if let Err(e) = fsx::valid_name(&fsx::file_name(b)) {
                self.toast(e);
                return;
            }
            if !seen.insert(b.clone()) {
                self.toast(tf!("Two files would both be called \"{}\" — add {{n}} to the pattern", fsx::file_name(b)));
                return;
            }
        }
        let mut done = Vec::new();
        let mut errs = Vec::new();
        for (a, b) in &plan {
            if b.exists() && !same_name_ignoring_case(a, b) {
                errs.push(tf!("\"{}\" already exists", fsx::file_name(b)));
                continue;
            }
            match std::fs::rename(a, b) {
                Ok(_) => done.push((a.clone(), b.clone())),
                Err(e) => errs.push(format!("{}: {}", fsx::file_name(a), fsx::friendly_err(&e))),
            }
        }
        for (a, b) in done.clone() {
            self.retag(&a, &b);
        }
        self.close_dialog();
        let dirs: Vec<PathBuf> = done.iter().filter_map(|(a, _)| a.parent().map(|x| x.to_path_buf())).collect();
        self.pane_mut().pending_select = done.iter().map(|(_, b)| b.clone()).collect();
        if !done.is_empty() {
            self.undo.push(UndoAct::Renamed(done.clone()));
        }
        self.on_fs_changed(dirs);
        if errs.is_empty() {
            self.toast(tf!("Renamed {} item{}", done.len(), if done.len() == 1 { "" } else { "s" }));
        } else {
            self.toast(tf!("Renamed {}, {} failed: {}", done.len(), errs.len(), errs[0]));
        }
    }

    // ── quick look ──
    fn quicklook_toggle(&mut self) {
        if self.g().get_ql() {
            self.quicklook_close();
            return;
        }
        if self.pane().selected.is_empty() {
            if let Some(c) = self.pane().cursor {
                self.move_cursor(0, false, Some(c));
            } else {
                return;
            }
        }
        self.quicklook_load();
    }

    fn quicklook_close(&mut self) {
        self.ql_gen.fetch_add(1, Ordering::SeqCst);
        self.g().set_ql(false);
        self.ui.invoke_focus_main();
    }

    fn quicklook_load(&mut self) {
        let sel = self.pane().selected_entries().into_iter().cloned().collect::<Vec<_>>();
        let Some(e) = sel.first().cloned() else { return };
        let gen = self.ql_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let cur = self.ql_gen.clone();
        let g = self.g();
        g.set_ql(true);
        g.set_ql_title(ss(&e.name));
        let sub = if e.is_dir { ts("Folder").to_string() } else { format!("{} · {} · {}", fsx::type_name(&e), fsx::fmt_size(e.size), fsx::fmt_time(e.modified)) };
        g.set_ql_sub(ss(sub));
        g.set_ql_kind(e.kind);
        g.set_ql_has_image(false);
        g.set_ql_has_text(false);
        let is_img = matches!(e.ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "jfif");
        let is_text = matches!(e.kind, fsx::K_CODE | fsx::K_TEXT | fsx::K_SHEET) && !matches!(e.ext.as_str(), "doc" | "docx" | "odt" | "pages" | "epub" | "ppt" | "pptx" | "odp" | "key" | "xls" | "xlsx" | "ods" | "numbers" | "rtf");
        if is_img && e.size < 120 * 1024 * 1024 {
            std::thread::spawn(move || {
                let Ok(img) = image::open(&e.path) else { return };
                if cur.load(Ordering::SeqCst) != gen {
                    return;
                }
                let t = img.thumbnail(2000, 2000).to_rgba8();
                let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(t.as_raw(), t.width(), t.height());
                let _ = slint::invoke_from_event_loop(move || {
                    with_app(move |a| {
                        if a.ql_gen.load(Ordering::SeqCst) == gen {
                            let g = a.g();
                            g.set_ql_image(slint::Image::from_rgba8(buf));
                            g.set_ql_has_image(true);
                        }
                    })
                });
            });
        } else if is_text && e.size < 16 * 1024 * 1024 {
            std::thread::spawn(move || {
                use std::io::Read;
                let Ok(mut f) = std::fs::File::open(&e.path) else { return };
                let mut buf = vec![0u8; 96 * 1024];
                let n = f.read(&mut buf).unwrap_or(0);
                buf.truncate(n);
                if buf.iter().take(4096).any(|&b| b == 0) {
                    return;
                }
                let text: String = String::from_utf8_lossy(&buf).lines().take(400).map(|l| l.replace('\t', "    ")).collect::<Vec<_>>().join("\n");
                let _ = slint::invoke_from_event_loop(move || {
                    with_app(move |a| {
                        if a.ql_gen.load(Ordering::SeqCst) == gen {
                            let g = a.g();
                            g.set_ql_text(ss(text));
                            g.set_ql_has_text(true);
                        }
                    })
                });
            });
        }
    }

    // ── drag & drop ──
    fn drop_target(&self, target: &str) -> Option<PathBuf> {
        if let Some(rest) = target.strip_prefix("::row:") {
            let (pi, idx) = rest.split_once(':')?;
            let e = self.tabs[self.cur].panes.get(pi.parse::<usize>().ok()?)?.entry(idx.parse().ok()?)?;
            return if e.is_dir { Some(e.path.clone()) } else { None };
        }
        if let Some(pi) = target.strip_prefix("::pane:") {
            let pi: usize = pi.parse().ok()?;
            return self.tabs[self.cur].panes.get(pi)?.loc.dir().map(|d| d.to_path_buf());
        }
        if let Some(i) = target.strip_prefix("::tab:") {
            let t = self.tabs.get(i.parse::<usize>().ok()?)?;
            return t.panes[t.active].loc.dir().map(|d| d.to_path_buf());
        }
        if target.starts_with("::") {
            return None;
        }
        let p = PathBuf::from(target);
        if p.is_dir() { Some(p) } else { None }
    }

    /// What a drop on `target` would do: 0 nothing, 1 copy, 2 move. `copy` = the user asked
    /// for a copy (Ctrl). Like Explorer: same drive moves, another drive copies.
    fn drop_action(&self, target: &str, data: &DataTransfer, copy: bool) -> i32 {
        let Ok(paths) = data.file_paths() else { return 0 };
        let paths: Vec<PathBuf> = paths.map(|p| p.to_path_buf()).collect();
        if paths.is_empty() {
            return 0;
        }
        if let Some(t) = target.strip_prefix("::tag:") {
            return if t.parse::<u8>().is_ok() { 1 } else { 0 };
        }
        if self.is_bin_target(target) {
            return if paths.iter().any(|p| self.bin_items.contains_key(p)) { 0 } else { 2 };
        }
        let Some(dest) = self.drop_target(target) else { return 0 };
        // not into itself / one of its children, and not a no-op move into the same folder
        if paths.iter().any(|p| dest.starts_with(p)) {
            return 0;
        }
        if paths.iter().all(|p| p.parent() == Some(dest.as_path())) {
            return 0;
        }
        let other_drive = paths.iter().any(|p| self.drive_for(p).map(|d| d.mount.clone()) != self.drive_for(&dest).map(|d| d.mount.clone()));
        if copy || other_drive { 1 } else { 2 }
    }

    fn is_bin_target(&self, target: &str) -> bool {
        if target == "::bin" {
            return true;
        }
        if let Some(pi) = target.strip_prefix("::pane:") {
            if let Ok(pi) = pi.parse::<usize>() {
                return self.tabs[self.cur].panes.get(pi).map(|p| p.loc == Loc::Bin).unwrap_or(false);
            }
        }
        false
    }

    fn drop_files(&mut self, target: &str, data: &DataTransfer, copy: bool) {
        let action = self.drop_action(target, data, copy);
        if action == 0 {
            return;
        }
        let Ok(paths) = data.file_paths() else { return };
        let paths: Vec<PathBuf> = paths.map(|p| p.to_path_buf()).collect();
        if self.is_bin_target(target) {
            self.do_delete(paths, false);
            return;
        }
        if let Some(t) = target.strip_prefix("::tag:") {
            let i: u8 = t.parse().unwrap_or(0);
            // dropping on a tag only ever adds it
            let bit = 1u8 << i;
            let missing: Vec<PathBuf> = paths.iter().filter(|p| self.tags.get(*p).map(|m| m & bit == 0).unwrap_or(true)).cloned().collect();
            if !missing.is_empty() {
                self.toggle_tag(i, missing);
            }
            return;
        }
        let Some(dest) = self.drop_target(target) else { return };
        let kind = if action == 1 { OpKind::Copy } else { OpKind::Move };
        self.start_op(kind, paths, Some(dest));
    }
}


// ───────────────────────── v0.4 features ─────────────────────────

impl App {
    // ── command palette ──
    fn shortcut_for(&self, action: &str) -> String {
        if let Some((_, c)) = self.s.shortcuts.iter().find(|(a, _)| a == action) {
            return c.clone();
        }
        DEFAULT_KEYS.iter().find(|(a, _)| *a == action).map(|(_, k)| k.to_string()).unwrap_or_default()
    }

    fn palette_items(&self) -> Vec<PalItem> {
        let p = |label: &str, icon: i32, action: &str| PalItem { label: i18n::t(label), en: label.to_string(), icon, action: action.to_string(), hint: String::new() };
        let mut v = vec![
            p("New folder", 36, "new-folder"),
            p("New file", 37, "new-file"),
            p("New tab", 42, "new-tab"),
            p("New window", 42, "new-window"),
            p("Close tab", 42, "close-tab"),
            p("Reopen closed tab", 56, "reopen-tab"),
            p("Cut", 31, "cut"),
            p("Copy", 32, "copy"),
            p("Paste", 33, "paste"),
            p("Copy path", 41, "copy-path"),
            p("Copy folder path", 41, "copy-folder-path"),
            p("Rename", 34, "rename"),
            p("Batch rename", 59, "batch-rename"),
            p("Move to the bin", 35, "trash"),
            p("Delete permanently", 35, "delete-perm"),
            p("Undo", 56, "undo"),
            p("Select all", 45, "select-all"),
            p("Invert selection", 45, "invert-sel"),
            p("Refresh", 43, "refresh"),
            p("Properties", 39, "properties"),
            p("Hashes (SHA-256, MD5, …)", 52, "hashes"),
            p("Verify a checksum file", 60, "verify-sums"),
            p("Compress to ZIP", 53, "compress"),
            p("Extract here", 54, "extract-here"),
            p("Quick look", 58, "quicklook"),
            p("Open in terminal", 38, "terminal"),
            p("Open in VS Code", 55, "vscode"),
            p("Details view", 50, "view:0"),
            p("Grid view", 49, "view:1"),
            p("Column view", 63, "view:2"),
            p("Small icons", 49, "grid:0"),
            p("Medium icons", 49, "grid:1"),
            p("Large icons", 49, "grid:2"),
            p("Sort by name", 48, "sort:0"),
            p("Sort by date modified", 48, "sort:1"),
            p("Sort by date created", 48, "sort:5"),
            p("Sort by type", 48, "sort:2"),
            p("Sort by size", 48, "sort:3"),
            p("Show / hide hidden files", 44, "toggle-hidden"),
            p("Show / hide file extensions", 44, "toggle-ext"),
            p("Preview pane", 44, "toggle-preview"),
            p("Dual pane", 63, "toggle-dual"),
            p("Friendly dates", 44, "toggle-friendly"),
            p("Go to Home", 15, "go-home"),
            p("Go back", 30, "go-back"),
            p("Go forward", 30, "go-forward"),
            p("Go up one folder", 30, "go-up"),
            p("Type a path", 41, "edit-path"),
            p("Search in this folder", 26, "focus-search"),
            p("Connect to server…", 28, "connect"),
            p("Settings", 46, "settings"),
            p("Keyboard shortcuts", 46, "settings-keys"),
            p("Customize the toolbar", 46, "settings-toolbar"),
            p("Language: English", 64, "lang:en"),
            p("Język: polski", 64, "lang:pl"),
            p("Language: automatic (system)", 64, "lang:auto"),
        ];
        if fsx::BIN_VIEW {
            v.push(p(&tf!("Go to the {}", fsx::trash_word()), 24, "open-bin"));
            v.push(p(&tf!("Empty the {}", fsx::trash_word()), 24, "empty-bin"));
        }
        if cfg!(windows) {
            v.push(p("Show more options (Windows menu)", 61, "shell-menu"));
        }
        if crate::sysint::supported() {
            v.push(p(if crate::sysint::is_default() { "Stop using SKJ Files as the default file manager" } else { "Make SKJ Files the default file manager" }, 46, "default-fm"));
        }
        for it in &mut v {
            it.hint = i18n::key_label(&self.shortcut_for(&it.action));
        }
        for (label, kind, path) in &self.quick {
            v.push(PalItem { label: tf!("Go to {}", i18n::place_label(label)), en: format!("Go to {}", label), icon: *kind, action: format!("goto:{}", path.display()), hint: path.display().to_string() });
        }
        for d in &self.drives {
            v.push(PalItem { label: tf!("Go to {}", d.label), en: String::new(), icon: if d.network { 28 } else { fsx::K_DRIVE }, action: format!("goto:{}", d.mount.display()), hint: d.mount.display().to_string() });
        }
        for r in self.s.recent.iter().take(10) {
            v.push(PalItem { label: tf!("Recent: {}", fsx::file_name(r)), en: String::new(), icon: fsx::K_FOLDER, action: format!("goto:{}", r.display()), hint: r.display().to_string() });
        }
        for (i, t) in self.tabs.iter().enumerate() {
            let (title, kind) = self.loc_title(&t.panes[t.active].loc);
            v.push(PalItem { label: tf!("Switch to tab: {}", title), en: format!("Switch to tab: {}", title), icon: kind, action: format!("tab:{}", i), hint: format!("Ctrl+{}", i + 1) });
        }
        v
    }

    fn palette_open(&mut self) {
        self.pal_all = self.palette_items();
        let g = self.g();
        g.set_pal_query(SharedString::default());
        g.set_pal(true);
        self.palette_filter(String::new());
    }

    fn palette_close(&mut self) {
        self.g().set_pal(false);
        self.ui.invoke_focus_main();
    }

    fn palette_filter(&mut self, q: String) {
        let q = q.trim().to_string();
        let mut scored: Vec<(i32, usize)> = self
            .pal_all
            .iter()
            .enumerate()
            .filter_map(|(i, it)| {
                if q.is_empty() {
                    return Some((-(i as i32), i));
                }
                fuzzy_score(&q, &it.label)
                    .or_else(|| if it.en.is_empty() || it.en == it.label { None } else { fuzzy_score(&q, &it.en).map(|s| s - 10) })
                    .or_else(|| fuzzy_score(&q, &it.hint).map(|s| s - 40))
                    .map(|s| (s, i))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        self.pal_view = scored.into_iter().map(|x| x.1).take(60).collect();
        let mut rows: Vec<MenuEntry> = self.pal_view.iter().map(|&i| {
            let it = &self.pal_all[i];
            MenuEntry { label: ss(&it.label), icon: it.icon, action: ss(&it.action), separator: false, danger: false, shortcut: ss(&it.hint), checked: false }
        }).collect();
        // typed a path: offer to open it
        let looks_like_path = q.starts_with('/') || q.starts_with('~') || q.starts_with("\\\\") || (q.len() >= 2 && q.as_bytes()[1] == b':');
        if looks_like_path {
            let p = fsx::expand_user(&q);
            rows.insert(0, MenuEntry { label: ss(tf!("Open {}", p.display())), icon: fsx::K_FOLDER, action: ss(format!("goto:{}", p.display())), separator: false, danger: false, shortcut: SharedString::default(), checked: false });
        }
        let g = self.g();
        g.set_pal_rows(ModelRc::new(VecModel::from(rows)));
        g.set_pal_index(0);
    }

    fn palette_run(&mut self, idx: i32) {
        let rows = self.g().get_pal_rows();
        let Some(row) = rows.row_data(idx.max(0) as usize) else { return };
        self.palette_close();
        let a = row.action.to_string();
        self.action(&a, false);
    }

    // ── column view ──
    fn col_listing(dir: &Path, hidden: bool) -> Vec<(String, i32, bool)> {
        let mut v: Vec<(String, i32, bool)> = fsx::list_dir(dir)
            .unwrap_or_default()
            .into_iter()
            .filter(|e| hidden || !e.hidden)
            .map(|e| (e.name, e.kind, e.is_dir))
            .collect();
        v.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| fsx::natural_cmp(&a.0, &b.0)));
        v
    }

    /// Column view: the folders above the current one, each with the next step highlighted.
    fn refresh_columns(&mut self, pi: usize) {
        if pi > 1 {
            return;
        }
        self.col_gen[pi] += 1;
        let gen = self.col_gen[pi];
        let empty = || ModelRc::new(VecModel::from(Vec::<ColData>::new()));
        let dir = if self.s.view_mode == 2 { self.tabs[self.cur].panes[pi].loc.dir().map(|d| d.to_path_buf()) } else { None };
        let Some(dir) = dir else {
            self.col_paths[pi].clear();
            if pi == 0 { self.g().set_p0_cols(empty()) } else { self.g().set_p1_cols(empty()) }
            return;
        };
        let hidden = self.s.show_hidden;
        let mut chain: Vec<PathBuf> = dir.ancestors().skip(1).take(8).map(|a| a.to_path_buf()).collect();
        chain.reverse();
        let tab_id = self.tabs[self.cur].id;
        std::thread::spawn(move || {
            let mut cols: Vec<(PathBuf, Vec<(String, i32, bool)>, Option<String>)> = Vec::new();
            for (i, a) in chain.iter().enumerate() {
                let next = chain.get(i + 1).unwrap_or(&dir);
                cols.push((a.clone(), App::col_listing(a, hidden), next.file_name().map(|n| n.to_string_lossy().to_string())));
            }
            let _ = slint::invoke_from_event_loop(move || with_app(move |a| a.on_columns(tab_id, pi, gen, cols)));
        });
    }

    fn on_columns(&mut self, tab_id: u64, pi: usize, gen: u64, cols: Vec<(PathBuf, Vec<(String, i32, bool)>, Option<String>)>) {
        if self.col_gen[pi] != gen || self.tabs[self.cur].id != tab_id {
            return;
        }
        let mut data = Vec::new();
        self.col_paths[pi] = cols.iter().map(|c| c.0.clone()).collect();
        self.col_rows[pi] = cols.iter().map(|c| c.1.iter().map(|r| r.0.clone()).collect()).collect();
        for (path, rows, sel) in cols {
            let title = if path.parent().is_none() { path.display().to_string() } else { fsx::file_name(&path) };
            let rows: Vec<ColRow> = rows
                .into_iter()
                .map(|(name, kind, dir)| ColRow { sel: sel.as_deref() == Some(name.as_str()), name: ss(name), kind, dir })
                .collect();
            let sel_i = rows.iter().position(|r| r.sel).map(|x| x as i32).unwrap_or(-1);
            data.push(ColData { title: ss(title), rows: ModelRc::new(VecModel::from(rows)), sel: sel_i });
        }
        let m = ModelRc::new(VecModel::from(data));
        if pi == 0 { self.g().set_p0_cols(m) } else { self.g().set_p1_cols(m) }
    }

    /// Column view: what's inside the selected folder, to the right of the current column.
    fn refresh_child_column(&mut self, pi: usize) {
        if pi > 1 {
            return;
        }
        self.col_gen[pi] += 0; // (columns keep their own gen)
        let sel: Vec<Entry> = self.tabs[self.cur].panes[pi].selected_entries().into_iter().cloned().collect();
        let target = if self.s.view_mode == 2 && sel.len() == 1 && sel[0].is_dir && self.tabs[self.cur].panes[pi].loc != Loc::Bin { Some(sel[0].path.clone()) } else { None };
        if target == self.child_dir[pi] && target.is_some() {
            return;
        }
        self.child_dir[pi] = target.clone();
        let set = move |a: &App, rows: Vec<ColRow>, title: String| {
            let g = a.g();
            let m = ModelRc::new(VecModel::from(rows));
            if pi == 0 {
                g.set_p0_child(m);
                g.set_p0_child_title(ss(title));
            } else {
                g.set_p1_child(m);
                g.set_p1_child_title(ss(title));
            }
        };
        let Some(dir) = target else {
            self.child_rows[pi].clear();
            set(self, Vec::new(), String::new());
            return;
        };
        let hidden = self.s.show_hidden;
        std::thread::spawn(move || {
            let rows = App::col_listing(&dir, hidden);
            let _ = slint::invoke_from_event_loop(move || {
                with_app(move |a| {
                    if a.child_dir[pi].as_ref() != Some(&dir) {
                        return;
                    }
                    a.child_rows[pi] = rows.iter().map(|r| r.0.clone()).collect();
                    let cr: Vec<ColRow> = rows.into_iter().map(|(name, kind, d)| ColRow { name: ss(name), kind, dir: d, sel: false }).collect();
                    set(a, cr, fsx::file_name(&dir));
                })
            });
        });
    }

    /// Click in an earlier column (or the child column, `ci` = -1): open that folder with the
    /// clicked item selected — exactly like Finder's columns.
    fn col_click(&mut self, pi: usize, ci: i32, ri: usize, new_tab: bool) {
        let (dir, name) = if ci < 0 {
            match (self.child_dir[pi].clone(), self.child_rows[pi].get(ri).cloned()) {
                (Some(d), Some(n)) => (d, n),
                _ => return,
            }
        } else {
            let ci = ci as usize;
            match (self.col_paths[pi].get(ci).cloned(), self.col_rows[pi].get(ci).and_then(|r| r.get(ri)).cloned()) {
                (Some(d), Some(n)) => (d, n),
                _ => return,
            }
        };
        let target = dir.join(&name);
        if new_tab {
            if target.is_dir() {
                self.open_tab(Loc::Dir(target), false);
            }
            return;
        }
        self.activate_pane(pi);
        self.navigate_select(pi, dir, target);
    }

    /// Go to `dir` and select `item` there (also when `dir` is already open).
    fn navigate_select(&mut self, pi: usize, dir: PathBuf, item: PathBuf) {
        let ti = self.cur;
        if self.tabs[ti].panes[pi].loc.dir() == Some(dir.as_path()) {
            let idx = {
                let p = &self.tabs[ti].panes[pi];
                p.view.iter().position(|&i| p.entries[i].path == item)
            };
            if let Some(idx) = idx {
                self.move_cursor(0, false, Some(idx));
            }
            return;
        }
        self.tabs[ti].panes[pi].pending_select = vec![item];
        self.navigate(pi, Loc::Dir(dir), true);
    }

    // ── tabs ──
    fn tab_move(&mut self, from: usize, to: usize) {
        if from >= self.tabs.len() || to >= self.tabs.len() || from == to {
            return;
        }
        let cur_id = self.tabs[self.cur].id;
        let t = self.tabs.remove(from);
        self.tabs.insert(to, t);
        self.cur = self.tab_index(cur_id).unwrap_or(0);
        self.refresh_titles();
        self.sync_tab_drags();
    }

    fn sync_tab_drags(&self) {
        let m = self.tab_drags_model.clone();
        while m.row_count() < self.tabs.len() {
            let mut d = DataTransfer::default();
            d.set_plain_text(SharedString::from(format!("::tab:{}", m.row_count())));
            m.push(d);
        }
        while m.row_count() > self.tabs.len() {
            m.remove(m.row_count() - 1);
        }
    }

    /// A drag over a tab: another tab (reorder) or files (into that tab's folder).
    fn tab_drop_action(&self, i: usize, data: &DataTransfer, copy: bool) -> i32 {
        if let Ok(t) = data.plain_text() {
            if t.starts_with("::tab:") {
                return 2;
            }
        }
        self.drop_action(&format!("::tab:{}", i), data, copy)
    }

    fn tab_dropped(&mut self, i: usize, data: &DataTransfer, copy: bool) {
        if let Ok(t) = data.plain_text() {
            if let Some(from) = t.strip_prefix("::tab:") {
                if let Ok(from) = from.parse::<usize>() {
                    self.tab_move(from, i);
                }
                return;
            }
        }
        self.drop_files(&format!("::tab:{}", i), data, copy);
    }

    fn tab_menu(&mut self, i: usize, x: f32, y: f32) {
        self.menu_tab = i;
        self.menu = MenuCtx { pane: self.ap(), targets: vec![], side_path: None };
        let n = self.tabs.len();
        let mut m = vec![mi("New tab", 42, "new-tab", "Ctrl+T"), mi("Duplicate tab", 32, "tab-dup", ""), mi("Move to a new window", 30, "tab-window", "")];
        m.push(sep());
        m.push(mi("Close tab", 35, "tab-close", "Ctrl+W"));
        if n > 1 {
            m.push(mi("Close other tabs", 35, "tab-close-others", ""));
        }
        if i + 1 < n {
            m.push(mi("Close tabs to the right", 35, "tab-close-right", ""));
        }
        if !self.closed_tabs.is_empty() {
            m.push(sep());
            m.push(mi("Reopen closed tab", 56, "reopen-tab", "Ctrl+Shift+T"));
        }
        self.show_menu(m, x, y);
    }

    fn tab_loc(&self, i: usize) -> Option<Loc> {
        self.tabs.get(i).map(|t| t.panes[t.active].loc.clone())
    }

    fn spawn_window(&self, loc: &Loc) {
        if let Ok(exe) = std::env::current_exe() {
            let mut c = std::process::Command::new(exe);
            if let Loc::Dir(d) = loc {
                c.arg(d);
            }
            let _ = c.spawn();
        }
    }

    // ── network ──
    fn open_connect(&mut self) {
        let g = self.g();
        g.set_cn_user(SharedString::default());
        g.set_cn_pass(SharedString::default());
        let hint = if cfg!(windows) { ts("\\\\server\\share  or  smb://server/share") } else { ts("smb://server/share  ·  sftp://user@host/folder") };
        self.open_dialog(Dialog::Connect, ts("Connect to server"), hint, "", 0, ts("Connect"), false);
    }

    fn connect_ok(&mut self, addr: String) {
        let Some(norm) = fsx::normalize_server(&addr) else {
            self.toast(if cfg!(windows) { ts("Type a server like \\\\nas\\photos") } else { ts("Type a server like smb://nas/photos") });
            return;
        };
        let g = self.g();
        let (user, pass) = (g.get_cn_user().to_string(), g.get_cn_pass().to_string());
        self.close_dialog();
        self.open_server(norm, user, pass, false);
    }

    fn open_server(&mut self, srv: String, user: String, pass: String, new_tab: bool) {
        self.toast(tf!("Connecting to {}…", fsx::server_label(&srv)));
        std::thread::spawn(move || {
            let r = fsx::connect_server(&srv, &user, &pass);
            let _ = slint::invoke_from_event_loop(move || {
                with_app(move |a| match r {
                    Ok(path) => {
                        if !a.s.servers.contains(&srv) {
                            a.s.servers.push(srv);
                            a.s.save();
                            a.sync_sidebar();
                        }
                        if new_tab {
                            a.open_tab(Loc::Dir(path), true);
                        } else {
                            let pi = a.ap();
                            a.navigate(pi, Loc::Dir(path), true);
                        }
                    }
                    Err(e) => a.toast(tf!("Couldn't connect: {}", e)),
                })
            });
        });
    }

    // ── recent files ──
    fn remember_file(&mut self, p: PathBuf) {
        if fsx::file_name(&p).to_lowercase().ends_with(".lnk") {
            return;
        }
        self.s.recent_files.retain(|x| x != &p);
        self.s.recent_files.insert(0, p);
        self.s.recent_files.truncate(30);
        self.s.save();
        self.refresh_recent_files();
    }

    fn refresh_recent_files(&mut self) {
        let ours = self.s.recent_files.clone();
        std::thread::spawn(move || {
            let mut list: Vec<(PathBuf, Option<std::time::SystemTime>)> = ours.into_iter().filter(|p| p.is_file()).map(|p| (p, None)).collect();
            for (p, t) in fsx::system_recent_files(20) {
                if !list.iter().any(|(x, _)| x == &p) {
                    list.push((p, Some(t)));
                }
            }
            list.truncate(12);
            let _ = slint::invoke_from_event_loop(move || {
                with_app(move |a| {
                    let items: Vec<SideItem> = list
                        .into_iter()
                        .map(|(p, t)| {
                            let name = fsx::file_name(&p);
                            let shown = name.strip_suffix(".lnk").unwrap_or(&name).to_string();
                            let kind = fsx::kind_for_ext(&fsx::ext_of(&shown));
                            let folder = if name.ends_with(".lnk") { String::new() } else { p.parent().map(|x| x.display().to_string()).unwrap_or_default() };
                            let when = t.map(|t| fsx::friendly_time(Some(t))).unwrap_or_default();
                            let sub = match (folder.is_empty(), when.is_empty()) {
                                (false, false) => format!("{} · {}", folder, when),
                                (false, true) => folder,
                                _ => when,
                            };
                            SideItem { label: ss(shown), kind, path: ss(format!("::recentfile:{}", p.display())), usage: -1.0, sub: ss(sub), color: Color::default() }
                        })
                        .collect();
                    a.g().set_recent_files(ModelRc::new(VecModel::from(items)));
                })
            });
        });
    }

    // ── shortcuts ──
    fn sync_shortcut_rows(&self) {
        let rows: Vec<MenuEntry> = REBINDABLE
            .iter()
            .map(|(a, label)| {
                let custom = self.s.shortcuts.iter().any(|(x, _)| x == a);
                MenuEntry { label: ss(ts(label)), icon: -1, action: ss(a), separator: false, danger: false, shortcut: ss(i18n::key_label(&self.shortcut_for(a))), checked: custom }
            })
            .collect();
        self.g().set_key_rows(ModelRc::new(VecModel::from(rows)));
    }

    fn shortcut_capture(&mut self, action: String) {
        let label = REBINDABLE.iter().find(|(a, _)| *a == action).map(|(_, l)| ts(l)).unwrap_or("");
        self.g().set_capturing(ss(label));
        self.capture_action = Some(action);
    }

    /// The keys pressed while Settings waits for a new shortcut.
    fn shortcut_key(&mut self, text: &str, ctrl: bool, shift: bool, alt: bool) {
        let Some(action) = self.capture_action.clone() else { return };
        use slint::platform::Key;
        let t = SharedString::from(text);
        if t == SharedString::from(Key::Escape) {
            self.capture_action = None;
            self.g().set_capturing(SharedString::default());
            return;
        }
        if !ctrl && !alt && !shift && (t == SharedString::from(Key::Backspace) || t == SharedString::from(Key::Delete)) {
            // clear: back to the built-in key (if any)
            self.s.shortcuts.retain(|(a, _)| a != &action);
            self.finish_capture(tf!("{} uses its normal shortcut again", REBINDABLE.iter().find(|(a, _)| *a == action).map(|x| ts(x.1)).unwrap_or("")));
            return;
        }
        let Some(combo) = combo_of(text, ctrl, shift, alt) else { return };
        // plain letters would break typing to jump to a file
        if !ctrl && !alt && combo.chars().count() == 1 {
            self.toast(ts("Use Ctrl or Alt with a letter (plain letters jump to files)"));
            return;
        }
        self.s.shortcuts.retain(|(a, c)| a != &action && c != &combo);
        self.s.shortcuts.push((action, combo.clone()));
        self.finish_capture(tf!("Shortcut set to {}", i18n::key_label(&combo)));
    }

    fn finish_capture(&mut self, msg: String) {
        self.capture_action = None;
        self.g().set_capturing(SharedString::default());
        self.s.save();
        self.sync_shortcut_rows();
        self.toast(msg);
    }

    // ── default file manager ──
    /// "auto", "en" or "pl": switch the whole UI without a restart.
    fn set_language(&mut self, setting: &str) {
        let setting = if matches!(setting, "en" | "pl") { setting } else { "auto" };
        self.s.lang = setting.to_string();
        self.s.save();
        let code = i18n::resolve(setting);
        apply_language(code);
        self.g().set_lang_setting(ss(setting));
        // everything Rust has written into the UI is in the old language: write it again
        self.rebuild_all();
        self.sync_settings_ui();
        self.sync_sidebar();
        self.sync_all();
        self.refresh_recent_files();
        self.sync_shortcut_rows();
        if matches!(self.dialog, Dialog::Settings) {
            let g = self.g();
            g.set_dialog_title(ss(ts("Settings")));
            g.set_dialog_ok_text(ss(ts("Done")));
        }
        if self.g().get_pal() {
            self.pal_all = self.palette_items();
            let q = self.g().get_pal_query().to_string();
            self.palette_filter(q);
        }
        self.toast(if code == "pl" { "Język: polski" } else { "Language: English" });
    }

    fn toggle_default_fm(&mut self) {
        let on = !crate::sysint::is_default();
        if on {
            let prev = crate::sysint::previous_default();
            if !prev.is_empty() {
                self.s.prev_fm = prev;
                self.s.save();
            }
        }
        match crate::sysint::set_default(on, &self.s.prev_fm) {
            Ok(()) => self.toast(if on {
                if cfg!(windows) { ts("Done — folders and Win+E now open in SKJ Files") } else { ts("Done — folders now open in SKJ Files") }
            } else if cfg!(windows) {
                ts("Folders open in File Explorer again")
            } else {
                ts("Folders open in your previous file manager again")
            }),
            Err(e) => self.toast(tf!("Couldn't change it: {}", e)),
        }
        self.g().set_is_default_fm(crate::sysint::is_default());
    }

    // ── Windows: Explorer's own menu ──
    #[cfg(windows)]
    fn shell_menu(&mut self, background: bool, at_cursor_row: bool) {
        use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use slint::winit_030::WinitWindowAccessor;
        let hwnd = self.ui.window().with_winit_window(|w| match w.window_handle().ok().map(|h| h.as_raw()) {
            Some(RawWindowHandle::Win32(h)) => Some(windows::Win32::Foundation::HWND(h.hwnd.get() as *mut core::ffi::c_void)),
            _ => None,
        });
        let Some(Some(hwnd)) = hwnd else { return };
        let (items, bg) = if background { (Vec::new(), self.current_dir()) } else if at_cursor_row { (self.targets(), None) } else { (self.menu.targets.clone(), None) };
        if items.is_empty() && bg.is_none() {
            return;
        }
        let sf = self.ui.window().scale_factor();
        let (x, y) = if at_cursor_row {
            let s = self.ui.window().size();
            (s.width as f32 / sf / 2.0, s.height as f32 / sf / 3.0)
        } else {
            self.menu_pos
        };
        let pi = self.ap();
        let first = items.first().cloned();
        let dir = self.current_dir();
        crate::winshell::show(hwnd, items, bg, (x * sf) as i32, (y * sf) as i32, false, move |outcome| {
            with_app(move |a| match outcome {
                crate::winshell::Outcome::Rename => {
                    if let Some(f) = first {
                        a.begin_rename(pi, f);
                    }
                }
                crate::winshell::Outcome::Done => {
                    // e.g. "Copy" from the shell menu: the clipboard changed under us
                    a.sync_commands();
                    if let Some(d) = dir {
                        slint::Timer::single_shot(Duration::from_millis(400), move || with_app(move |a| a.on_fs_changed(vec![d])));
                    }
                }
                crate::winshell::Outcome::Nothing => {}
            })
        });
    }
}

/// "Ctrl+Shift+K" style name for a key press, or None for keys that can't be shortcuts.
fn combo_of(text: &str, ctrl: bool, shift: bool, alt: bool) -> Option<String> {
    use slint::platform::Key;
    let t = SharedString::from(text);
    let named: &[(Key, &str)] = &[
        (Key::F1, "F1"), (Key::F2, "F2"), (Key::F3, "F3"), (Key::F4, "F4"), (Key::F5, "F5"), (Key::F6, "F6"),
        (Key::F7, "F7"), (Key::F8, "F8"), (Key::F9, "F9"), (Key::F10, "F10"), (Key::F11, "F11"), (Key::F12, "F12"),
        (Key::Delete, "Del"), (Key::Backspace, "Backspace"), (Key::Return, "Enter"), (Key::Tab, "Tab"),
        (Key::LeftArrow, "Left"), (Key::RightArrow, "Right"), (Key::UpArrow, "Up"), (Key::DownArrow, "Down"),
        (Key::Home, "Home"), (Key::End, "End"), (Key::PageUp, "PageUp"), (Key::PageDown, "PageDown"), (Key::Insert, "Insert"),
    ];
    let name = if let Some((_, n)) = named.iter().find(|(k, _)| t == SharedString::from(k.clone())) {
        n.to_string()
    } else if text == " " {
        "Space".to_string()
    } else {
        let mut c = text.chars();
        let ch = c.next()?;
        if c.next().is_some() || ch.is_control() || (ch as u32) >= 0xF700 {
            return None; // modifier-only presses, Escape, …
        }
        ch.to_uppercase().to_string()
    };
    let mut s = String::new();
    if ctrl {
        s.push_str("Ctrl+");
    }
    if shift {
        s.push_str("Shift+");
    }
    if alt {
        s.push_str("Alt+");
    }
    s.push_str(&name);
    Some(s)
}

/// Subsequence match with bonuses for word starts and runs; None if the letters aren't all there.
/// Lowercase without Polish diacritics, so "jezyk" finds "Język".
fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'ą' => 'a',
            'ć' => 'c',
            'ę' => 'e',
            'ł' => 'l',
            'ń' => 'n',
            'ó' => 'o',
            'ś' => 's',
            'ź' | 'ż' => 'z',
            _ => c,
        })
        .collect()
}

fn fuzzy_score(q: &str, s: &str) -> Option<i32> {
    let qf = fold(q.trim());
    let q: Vec<char> = qf.chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some(0);
    }
    let sf = fold(s);
    // the typed text as one piece ("kosz" in "Opróżnij kosz") beats letters scattered around
    let whole = sf.find(&qf).map(|at| {
        let word_start = at == 0 || !sf[..at].chars().last().map(|c| c.is_alphanumeric()).unwrap_or(false);
        40 * q.len() as i32 + if word_start { 30 } else { 0 } + if at == 0 { 20 } else { 0 } - sf.chars().count() as i32 / 4
    });
    let sl: Vec<char> = sf.chars().collect();
    let mut score = 0;
    let mut qi = 0;
    let mut prev_match = false;
    for (i, &c) in sl.iter().enumerate() {
        if qi < q.len() && c == q[qi] {
            let word_start = i == 0 || !sl[i - 1].is_alphanumeric();
            score += 10 + if word_start { 15 } else { 0 } + if prev_match { 12 } else { 0 };
            if i == qi {
                score += 8; // matches from the very beginning
            }
            qi += 1;
            prev_match = true;
        } else {
            prev_match = false;
            score -= 1;
        }
    }
    if qi == q.len() { Some(whole.map_or(score, |w| w.max(score))) } else { whole }
}

fn plural(n: usize, word: &str) -> String {
    i18n::count_word(n, word)
}

/// "name" in English, „name” in Polish.
fn quoted(s: &str) -> String {
    if i18n::pl() { format!("„{}”", s) } else { format!("\"{}\"", s) }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn same_name_ignoring_case(a: &Path, b: &Path) -> bool {
    cfg!(any(windows, target_os = "macos")) && a.display().to_string().to_lowercase() == b.display().to_string().to_lowercase()
}

fn mi(label: &str, icon: i32, action: &str, shortcut: &str) -> MenuEntry {
    MenuEntry { label: ss(i18n::t(label)), icon, action: ss(action), separator: false, danger: false, shortcut: ss(i18n::key_label(shortcut)), checked: false }
}

fn chk(label: &str, action: &str, checked: bool) -> MenuEntry {
    MenuEntry { label: ss(i18n::t(label)), icon: -1, action: ss(action), separator: false, danger: false, shortcut: SharedString::default(), checked }
}

fn tags_row() -> MenuEntry {
    MenuEntry { label: ss(ts("Tags")), icon: 57, action: ss("::tags"), separator: false, danger: false, shortcut: SharedString::default(), checked: false }
}

fn sep() -> MenuEntry {
    MenuEntry { label: SharedString::default(), icon: -1, action: SharedString::default(), separator: true, danger: false, shortcut: SharedString::default(), checked: false }
}

// ───────────────────────── wiring ─────────────────────────

pub fn wire(ui: &AppWindow) {
    let g = ui.global::<S>();
    g.on_tab_select(|i| with_app(move |a| a.select_tab(i as usize)));
    g.on_tab_close(|i| with_app(move |a| a.close_tab(i as usize)));
    g.on_tab_new(|| with_app(|a| a.open_tab(Loc::Home, true)));
    g.on_pane_activate(|i| with_app(move |a| a.activate_pane(i as usize)));
    g.on_nav_back(|| with_app(|a| a.go_back()));
    g.on_nav_forward(|| with_app(|a| a.go_forward()));
    g.on_nav_up(|| with_app(|a| a.go_up()));
    g.on_refresh(|| with_app(|a| a.refresh()));
    g.on_open_path(|p| with_app(move |a| a.open_path_str(&p, false)));
    g.on_open_path_new_tab(|p| with_app(move |a| a.open_path_str(&p, true)));
    g.on_crumb_clicked(|i| {
        with_app(move |a| {
            let i = i as usize;
            let loc = a.pane().loc.clone();
            if let Some(p) = a.crumb_paths.get(i).cloned() {
                let pi = a.ap();
                match loc {
                    Loc::Search { .. } if i == 1 => {}
                    _ => a.navigate(pi, Loc::Dir(p), true),
                }
            }
        })
    });
    g.on_path_entered(|t| with_app(move |a| a.path_entered(&t)));
    g.on_search_changed(|t| with_app(move |a| a.search_changed(t.to_string())));
    g.on_search_submitted(|t| with_app(move |a| a.search_submitted(t.to_string())));
    g.on_row_down(|p, i, c, s| with_app(move |a| a.row_down(p as usize, i as usize, c, s)));
    g.on_row_right(|p, i, x, y| with_app(move |a| a.row_right(p as usize, i as usize, x, y)));
    g.on_row_middle(|p, i| with_app(move |a| a.open_index(p as usize, i as usize, true)));
    g.on_row_open(|p, i| with_app(move |a| a.open_index(p as usize, i as usize, false)));
    g.on_bg_down(|p, keep| with_app(move |a| a.bg_down(p as usize, keep)));
    g.on_bg_menu(|p, x, y| with_app(move |a| a.bg_menu(p as usize, x, y)));
    g.on_sort_by(|k| with_app(move |a| a.sort_by(k)));
    g.on_cmd(|c| with_app(move |a| a.action(&c, false)));
    g.on_cmd_at(|c, x, y| with_app(move |a| a.cmd_at(&c, x, y)));
    g.on_menu_action(|c| with_app(move |a| a.action(&c, true)));
    g.on_side_menu(|p, x, y| with_app(move |a| a.side_menu(p.to_string(), x, y)));
    g.on_dialog_ok(|t| with_app(move |a| a.dialog_ok(t.to_string())));
    g.on_dialog_cancel(|| with_app(|a| a.close_dialog()));
    g.on_dialog_aux_clicked(|| with_app(|a| a.dialog_aux()));
    g.on_key(|t, c, s, al| with_app_ret(move |a| a.key(&t, c, s, al)));
    g.on_set_theme(|d| with_app(move |a| a.set_theme(d)));
    g.on_set_accent(|i| with_app(move |a| a.set_accent(i as usize)));
    g.on_toggle_setting(|w| with_app(move |a| a.toggle_setting(&w)));
    g.on_hash_compare_changed(|t| with_app(move |a| a.hash_compare(t.to_string())));
    g.on_hash_copy(|w| with_app(move |a| a.hash_copy(w.to_string())));
    g.on_batch_changed(|| with_app(|a| a.batch_preview()));
    g.on_can_drop_at(|t, d, c| with_app_ret(move |a| a.drop_action(&t, &d, c)));
    g.on_drop_at(|t, d, c| with_app(move |a| a.drop_files(&t, &d, c)));
    g.on_ql_close(|| with_app(|a| a.quicklook_close()));
    g.on_ql_open(|| with_app(|a| a.open_selected()));
    g.on_row_click(|p, i| with_app(move |a| a.row_click(p as usize, i as usize)));
    g.on_rename_commit(|t, enter| with_app(move |a| a.rename_commit(t.to_string(), enter)));
    g.on_marquee_begin(|p, add| with_app(move |a| a.marquee_begin(p as usize, add)));
    g.on_marquee(|p, x0, y0, x1, y1, cols, cw, ch, iw, ih, pad| with_app(move |a| a.marquee_update(p as usize, x0, y0, x1, y1, cols, cw, ch, iw, ih, pad)));
    g.on_marquee_end(|| with_app(|a| a.marquee_end()));
    g.on_conflict_choose(|w| with_app(move |a| a.conflict_choose(w)));
    g.on_col_resized(|| with_app(|a| a.col_resized()));
    g.on_header_menu(|x, y| with_app(move |a| a.header_menu(x, y)));
    g.on_pal_changed(|q| with_app(move |a| a.palette_filter(q.to_string())));
    g.on_pal_run(|i| with_app(move |a| a.palette_run(i)));
    g.on_pal_close(|| with_app(|a| a.palette_close()));
    g.on_col_click(|p, c, r, nt| with_app(move |a| a.col_click(p as usize, c, r as usize, nt)));
    g.on_tab_menu(|i, x, y| with_app(move |a| a.tab_menu(i as usize, x, y)));
    g.on_tab_can_drop(|i, d, c| with_app_ret(move |a| a.tab_drop_action(i as usize, &d, c)));
    g.on_tab_drop(|i, d, c| with_app(move |a| a.tab_dropped(i as usize, &d, c)));
    g.on_shortcut_capture(|act| with_app(move |a| a.shortcut_capture(act.to_string())));
    g.on_shortcut_key(|t, c, s, al| with_app(move |a| a.shortcut_key(&t, c, s, al)));
}
