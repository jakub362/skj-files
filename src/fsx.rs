//! File-system helpers: listing, kinds, formatting, drives, file operations, search.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::time::{Instant, SystemTime};

use crate::i18n::{self, ts};
use crate::tf;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub hidden: bool,
    pub kind: i32,
    pub ext: String,
    /// Parent folder, only filled for search results.
    pub location: String,
}

pub const K_FOLDER: i32 = 0;
pub const K_FILE: i32 = 1;
pub const K_IMAGE: i32 = 2;
pub const K_VIDEO: i32 = 3;
pub const K_AUDIO: i32 = 4;
pub const K_ARCHIVE: i32 = 5;
pub const K_CODE: i32 = 6;
pub const K_TEXT: i32 = 7;
pub const K_SHEET: i32 = 8;
pub const K_APP: i32 = 9;
pub const K_DISC: i32 = 10;
pub const K_PDF: i32 = 11;
pub const K_LINK: i32 = 12;
pub const K_DRIVE: i32 = 13;
pub const K_USB: i32 = 14;
pub const K_HOME: i32 = 15;
pub const K_DESKTOP: i32 = 16;
pub const K_DOWNLOADS: i32 = 17;
pub const K_PICTURES: i32 = 18;
pub const K_MUSIC: i32 = 19;
pub const K_VIDEOS: i32 = 20;
pub const K_DOCUMENTS: i32 = 21;
pub const K_USER: i32 = 22;
pub const K_CLOUD: i32 = 23;
pub const K_TRASH: i32 = 24;
pub const K_SEARCH: i32 = 26;

pub fn kind_for_ext(ext: &str) -> i32 {
    match ext {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg" | "tif" | "tiff" | "heic" | "heif"
        | "avif" | "raw" | "cr2" | "nef" | "arw" | "dng" | "psd" | "xcf" | "jfif" => K_IMAGE,
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "webm" | "flv" | "m4v" | "mpg" | "mpeg" | "3gp" | "m2ts" | "mts" => K_VIDEO,
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "wma" | "opus" | "mid" | "midi" | "aiff" | "aif" => K_AUDIO,
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" | "lz4" | "tgz" | "cab" | "lzma" | "tbz2" | "txz" => K_ARCHIVE,
        "rs" | "py" | "js" | "mjs" | "cjs" | "ts" | "jsx" | "tsx" | "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "hh" | "cs"
        | "java" | "kt" | "kts" | "go" | "rb" | "php" | "html" | "htm" | "css" | "scss" | "sass" | "less" | "json"
        | "xml" | "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf" | "sh" | "bash" | "zsh" | "fish" | "bat" | "cmd"
        | "ps1" | "lua" | "luau" | "sql" | "slint" | "vue" | "svelte" | "swift" | "dart" | "cmake" | "mk" | "gradle"
        | "ino" | "asm" | "s" | "r" | "pl" | "zig" | "nim" | "hs" | "ex" | "exs" | "erl" | "scala" | "m" | "mm"
        | "vb" | "fs" | "glsl" | "hlsl" | "wgsl" | "lock" | "gitignore" | "dockerfile" | "makefile" | "ipynb" => K_CODE,
        "txt" | "md" | "markdown" | "log" | "rtf" | "doc" | "docx" | "odt" | "pages" | "tex" | "nfo" | "srt" | "vtt"
        | "epub" | "ppt" | "pptx" | "odp" | "key" => K_TEXT,
        "xls" | "xlsx" | "ods" | "csv" | "tsv" | "numbers" => K_SHEET,
        "pdf" => K_PDF,
        "exe" | "msi" | "appimage" | "apk" | "aab" | "deb" | "rpm" | "dmg" | "app" | "com" | "jar" | "flatpakref" | "pkg" => K_APP,
        "iso" | "img" | "vhd" | "vhdx" | "vmdk" | "vdi" | "qcow2" | "wim" | "esd" => K_DISC,
        "lnk" | "url" | "desktop" | "webloc" => K_LINK,
        _ => K_FILE,
    }
}

pub fn type_name(e: &Entry) -> String {
    if e.is_dir {
        return ts("Folder").into();
    }
    let up = e.ext.to_uppercase();
    let named = match e.ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "heic" | "avif" | "jfif" => Some(tf!("{} image", up)),
        "svg" => Some(ts("SVG image").into()),
        "ico" => Some(ts("Icon").into()),
        "psd" => Some(ts("Photoshop image").into()),
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "webm" | "flv" | "m4v" | "mpg" | "mpeg" => Some(tf!("{} video", up)),
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "wma" | "opus" => Some(tf!("{} audio", up)),
        "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "zst" | "lz4" | "tgz" | "cab" => Some(tf!("{} archive", up)),
        "rs" => Some(ts("Rust source").into()),
        "py" => Some(ts("Python script").into()),
        "js" | "mjs" | "cjs" => Some("JavaScript".into()),
        "ts" if e.kind == K_CODE => Some("TypeScript".into()),
        "tsx" | "jsx" => Some(tf!("{} component", up)),
        "c" => Some(ts("C source").into()),
        "h" | "hpp" | "hh" => Some(ts("C/C++ header").into()),
        "cpp" | "cc" | "cxx" => Some(ts("C++ source").into()),
        "cs" => Some(ts("C# source").into()),
        "java" => Some(ts("Java source").into()),
        "kt" | "kts" => Some(ts("Kotlin source").into()),
        "go" => Some(ts("Go source").into()),
        "html" | "htm" => Some(ts("HTML document").into()),
        "css" => Some(ts("Stylesheet").into()),
        "json" => Some(ts("JSON file").into()),
        "xml" => Some(ts("XML file").into()),
        "yaml" | "yml" => Some(ts("YAML file").into()),
        "toml" => Some(ts("TOML file").into()),
        "ini" | "cfg" | "conf" => Some(ts("Settings file").into()),
        "sh" | "bash" | "zsh" => Some(ts("Shell script").into()),
        "bat" | "cmd" => Some(ts("Batch file").into()),
        "ps1" => Some(ts("PowerShell script").into()),
        "lua" | "luau" => Some(ts("Lua script").into()),
        "sql" => Some(ts("SQL file").into()),
        "slint" => Some(ts("Slint UI").into()),
        "ino" => Some(ts("Arduino sketch").into()),
        "txt" => Some(ts("Text document").into()),
        "md" | "markdown" => Some("Markdown".into()),
        "log" => Some(ts("Log file").into()),
        "doc" | "docx" => Some(ts("Word document").into()),
        "odt" => Some(ts("OpenDocument text").into()),
        "ppt" | "pptx" => Some("PowerPoint".into()),
        "xls" | "xlsx" => Some(ts("Excel workbook").into()),
        "ods" => Some(ts("OpenDocument sheet").into()),
        "csv" => Some(ts("CSV table").into()),
        "pdf" => Some(ts("PDF document").into()),
        "exe" => Some(ts("Application").into()),
        "msi" => Some(ts("Windows installer").into()),
        "appimage" => Some("AppImage".into()),
        "apk" => Some(ts("Android app").into()),
        "deb" | "rpm" => Some(tf!("{} package", up)),
        "dmg" => Some(ts("Disk image").into()),
        "jar" => Some(ts("Java archive").into()),
        "iso" => Some(ts("Disc image").into()),
        "img" | "vhd" | "vhdx" | "vmdk" | "vdi" | "qcow2" => Some(ts("Disk image").into()),
        "lnk" => Some(ts("Shortcut").into()),
        "url" => Some(ts("Internet shortcut").into()),
        "desktop" => Some(ts("App launcher").into()),
        "dll" => Some(ts("Library").into()),
        "so" => Some(ts("Shared library").into()),
        "ttf" | "otf" | "woff" | "woff2" => Some(ts("Font").into()),
        "torrent" => Some("Torrent".into()),
        _ => None,
    };
    named.unwrap_or_else(|| if e.ext.is_empty() { ts("File").into() } else { tf!("{} file", up) })
}

pub fn ext_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name[i + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

#[cfg(windows)]
fn is_hidden(name: &str, md: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    let attr = md.file_attributes();
    let _ = name;
    (attr & 0x2) != 0 || (attr & 0x4) != 0
}

#[cfg(not(windows))]
fn is_hidden(name: &str, _md: &fs::Metadata) -> bool {
    name.starts_with('.')
}

pub fn entry_from(path: PathBuf, name: String, md: &fs::Metadata, location: String) -> Entry {
    let is_dir = md.is_dir();
    let ext = if is_dir { String::new() } else { ext_of(&name) };
    let kind = if is_dir { K_FOLDER } else { kind_for_ext(&ext) };
    Entry {
        hidden: is_hidden(&name, md),
        size: if is_dir { 0 } else { md.len() },
        modified: md.modified().ok(),
        created: md.created().ok(),
        is_dir,
        kind,
        ext,
        name,
        path,
        location,
    }
}

pub fn list_dir(dir: &Path) -> Result<Vec<Entry>, String> {
    let rd = fs::read_dir(dir).map_err(|e| friendly_err(&e))?;
    let mut out = Vec::with_capacity(256);
    for de in rd.flatten() {
        let name = de.file_name().to_string_lossy().into_owned();
        let path = de.path();
        let md = match de.metadata() {
            Ok(m) if m.file_type().is_symlink() => fs::metadata(&path).unwrap_or(m),
            Ok(m) => m,
            Err(_) => match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            },
        };
        out.push(entry_from(path, name, &md, String::new()));
    }
    Ok(out)
}

pub fn friendly_err(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::PermissionDenied => ts("Access denied").into(),
        io::ErrorKind::NotFound => ts("This location doesn't exist anymore").into(),
        _ => e.to_string(),
    }
}

// ───────────── formatting ─────────────

pub fn fmt_size(n: u64) -> String {
    const U: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if n < 1024 {
        return format!("{} B", n);
    }
    let mut v = n as f64 / 1024.0;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if v >= 100.0 {
        format!("{:.0} {}", v, U[i])
    } else if i18n::pl() {
        format!("{:.1} {}", v, U[i]).replace('.', ",")
    } else {
        format!("{:.1} {}", v, U[i])
    }
}

pub fn fmt_time(t: Option<SystemTime>) -> String {
    match t {
        Some(t) => {
            let dt: chrono::DateTime<chrono::Local> = t.into();
            dt.format(if i18n::pl() { "%d.%m.%Y %H:%M" } else { "%Y-%m-%d %H:%M" }).to_string()
        }
        None => String::new(),
    }
}

pub fn fmt_count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(if i18n::pl() { '\u{a0}' } else { ',' });
        }
        out.push(c);
    }
    out
}

/// Natural, case-insensitive compare: "file2" < "file10".
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) => {
                if ca.is_ascii_digit() && cb.is_ascii_digit() {
                    let mut na = String::new();
                    while let Some(&c) = ai.peek() {
                        if c.is_ascii_digit() { na.push(c); ai.next(); } else { break; }
                    }
                    let mut nb = String::new();
                    while let Some(&c) = bi.peek() {
                        if c.is_ascii_digit() { nb.push(c); bi.next(); } else { break; }
                    }
                    let ta = na.trim_start_matches('0');
                    let tb = nb.trim_start_matches('0');
                    let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                    if o != Ordering::Equal {
                        return o;
                    }
                } else {
                    let la = ca.to_lowercase().next().unwrap_or(ca);
                    let lb = cb.to_lowercase().next().unwrap_or(cb);
                    if la != lb {
                        return la.cmp(&lb);
                    }
                    ai.next();
                    bi.next();
                }
            }
        }
    }
}

// ───────────── places ─────────────

#[derive(Clone, Debug, PartialEq)]
pub struct Drive {
    pub label: String,
    pub mount: PathBuf,
    pub total: u64,
    pub free: u64,
    pub removable: bool,
    /// a network share (SMB, NFS, SSH, …)
    pub network: bool,
}

fn is_network_fs(fs: &str) -> bool {
    let f = fs.to_ascii_lowercase();
    ["cifs", "smb", "smbfs", "smb2", "smb3", "nfs", "nfs4", "afpfs", "webdav", "davfs", "sshfs", "fuse.sshfs", "fuse.rclone", "fuse.gvfsd-fuse", "9p"]
        .iter()
        .any(|x| f == *x || f.starts_with(&format!("{}.", x)))
}

pub fn list_drives() -> Vec<Drive> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut out: Vec<Drive> = Vec::new();
    for d in disks.list() {
        let mount = d.mount_point().to_path_buf();
        let ms = mount.to_string_lossy().to_string();
        let network = is_network_fs(&d.file_system().to_string_lossy()) || is_remote_drive(&mount);
        if !network && !keep_mount(&ms) {
            continue;
        }
        if out.iter().any(|x| x.mount == mount) {
            continue;
        }
        let name = d.name().to_string_lossy().to_string();
        let label = drive_label(&name, &ms, d.is_removable());
        out.push(Drive { label, mount, total: d.total_space(), free: d.available_space(), removable: d.is_removable(), network });
    }
    // Linux: shares mounted through GNOME/gvfs (gio mount, file managers) live here
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let uid = std::fs::metadata("/proc/self").map(|m| { use std::os::unix::fs::MetadataExt; m.uid() }).unwrap_or(1000);
        if let Ok(rd) = fs::read_dir(format!("/run/user/{}/gvfs", uid)) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                let label = gvfs_label(&name);
                out.push(Drive { label, mount: e.path(), total: 0, free: 0, removable: false, network: true });
            }
        }
    }
    #[cfg(windows)]
    if out.is_empty() {
        // fallback: probe drive letters directly
        for c in b'C'..=b'Z' {
            let root = PathBuf::from(format!("{}:\\", c as char));
            if root.is_dir() {
                let network = is_remote_drive(&root);
                out.push(Drive { label: tf!("Drive ({}:)", c as char), mount: root, total: 0, free: 0, removable: false, network });
            }
        }
    }
    out.sort_by(|a, b| a.mount.cmp(&b.mount));
    out
}

#[cfg(windows)]
fn keep_mount(_m: &str) -> bool {
    true
}

/// Mapped network drive (Z: → \\server\share)?
#[cfg(windows)]
fn is_remote_drive(root: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    let s = root.display().to_string();
    if s.starts_with("\\\\") {
        return true;
    }
    let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe { windows::Win32::Storage::FileSystem::GetDriveTypeW(PCWSTR(wide.as_ptr())) == 4 } // DRIVE_REMOTE
}

#[cfg(not(windows))]
fn is_remote_drive(_root: &Path) -> bool {
    false
}

/// "smb-share:server=nas,share=photos" → "photos on nas"
#[cfg(all(unix, not(target_os = "macos")))]
fn gvfs_label(name: &str) -> String {
    let (kind, rest) = name.split_once(':').unwrap_or(("", name));
    let mut server = "";
    let mut share = "";
    for kv in rest.split(',') {
        if let Some(v) = kv.strip_prefix("server=").or_else(|| kv.strip_prefix("host=")) {
            server = v;
        }
        if let Some(v) = kv.strip_prefix("share=") {
            share = v;
        }
    }
    let _ = kind;
    match (share.is_empty(), server.is_empty()) {
        (false, false) => tf!("{} on {}", share, server),
        (true, false) => server.to_string(),
        _ => name.to_string(),
    }
}

#[cfg(windows)]
fn drive_label(name: &str, mount: &str, removable: bool) -> String {
    let letter = mount.trim_end_matches('\\');
    let base = if !name.is_empty() { name.to_string() } else if removable { ts("USB Drive").into() } else { ts("Local Disk").into() };
    format!("{} ({})", base, letter)
}

#[cfg(target_os = "macos")]
fn keep_mount(m: &str) -> bool {
    m == "/" || (m.starts_with("/Volumes/") && !m.starts_with("/System"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn keep_mount(m: &str) -> bool {
    if m == "/" || m == "/home" {
        return true;
    }
    for p in ["/run/media/", "/media/", "/mnt/"] {
        if m.starts_with(p) {
            return true;
        }
    }
    false
}

#[cfg(not(windows))]
fn drive_label(name: &str, mount: &str, _removable: bool) -> String {
    if mount == "/" {
        return ts("File System").into();
    }
    if mount == "/home" {
        return ts("Home partition").into();
    }
    let last = Path::new(mount).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    if !last.is_empty() { last } else if !name.is_empty() { name.to_string() } else { mount.to_string() }
}

pub struct Place {
    pub label: String,
    pub kind: i32,
    pub path: PathBuf,
}

pub fn default_quick_access() -> Vec<Place> {
    let mut v = Vec::new();
    let mut push = |p: Option<PathBuf>, kind: i32| {
        if let Some(p) = p {
            if p.is_dir() && !v.iter().any(|x: &Place| x.path == p) {
                let label = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string());
                v.push(Place { label, kind, path: p });
            }
        }
    };
    push(dirs::home_dir(), K_USER);
    push(dirs::desktop_dir(), K_DESKTOP);
    push(dirs::download_dir(), K_DOWNLOADS);
    push(dirs::document_dir(), K_DOCUMENTS);
    push(dirs::picture_dir(), K_PICTURES);
    push(dirs::audio_dir(), K_MUSIC);
    push(dirs::video_dir(), K_VIDEOS);
    if let Some(h) = dirs::home_dir() {
        let od = h.join("OneDrive");
        if od.is_dir() {
            push(Some(od), K_CLOUD);
        }
    }
    v
}

pub fn kind_for_place(p: &Path) -> i32 {
    let eq = |o: Option<PathBuf>| o.map(|x| x == p).unwrap_or(false);
    if eq(dirs::home_dir()) { K_USER }
    else if eq(dirs::desktop_dir()) { K_DESKTOP }
    else if eq(dirs::download_dir()) { K_DOWNLOADS }
    else if eq(dirs::document_dir()) { K_DOCUMENTS }
    else if eq(dirs::picture_dir()) { K_PICTURES }
    else if eq(dirs::audio_dir()) { K_MUSIC }
    else if eq(dirs::video_dir()) { K_VIDEOS }
    else { K_FOLDER }
}

// ───────────── names ─────────────

pub fn valid_name(name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() {
        return Err(ts("The name can't be empty").into());
    }
    if n == "." || n == ".." {
        return Err(ts("That name isn't allowed").into());
    }
    #[cfg(windows)]
    let bad: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    #[cfg(not(windows))]
    let bad: &[char] = &['/', '\0'];
    if let Some(c) = n.chars().find(|c| bad.contains(c)) {
        return Err(tf!("A name can't contain {}", c));
    }
    Ok(())
}

/// "name.ext" → first free of "name.ext", "name - Copy.ext", "name - Copy (2).ext" …
pub fn unique_copy_name(dir: &Path, name: &str, is_dir: bool) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = split_name(name, is_dir);
    for i in 1.. {
        let candidate = if i == 1 { tf!("{} - Copy{}", stem, ext) } else { tf!("{} - Copy ({}){}", stem, i, ext) };
        let p = dir.join(&candidate);
        if !p.exists() {
            return p;
        }
    }
    unreachable!()
}

/// "New folder" → "New folder (2)" …
pub fn unique_new_name(dir: &Path, name: &str, is_dir: bool) -> String {
    if !dir.join(name).exists() {
        return name.to_string();
    }
    let (stem, ext) = split_name(name, is_dir);
    for i in 2.. {
        let c = format!("{} ({}){}", stem, i, ext);
        if !dir.join(&c).exists() {
            return c;
        }
    }
    unreachable!()
}

pub fn split_name(name: &str, is_dir: bool) -> (String, String) {
    if is_dir {
        return (name.to_string(), String::new());
    }
    match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_string(), name[i..].to_string()),
        _ => (name.to_string(), String::new()),
    }
}

// ───────────── file operations ─────────────

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum OpKind {
    Copy,
    Move,
    Trash,
    Delete,
    Zip,
    Extract,
}

/// What to do when an item with the same name already exists in the destination.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Conflict {
    /// Give the new item a " - Copy" name.
    KeepBoth,
    /// Old item goes to the bin, new one takes its place. Folders are merged instead.
    Replace,
    /// Leave the existing item alone and don't transfer this one.
    Skip,
}

#[derive(Default)]
pub struct OpResult {
    pub done: usize,
    pub created: Vec<PathBuf>,
    /// (from, to) pairs for moves, so they can be undone
    pub pairs: Vec<(PathBuf, PathBuf)>,
    pub errors: Vec<String>,
    pub skipped: usize,
    /// Existing items that were replaced (their old version went to the bin).
    pub replaced: usize,
    /// At least one folder was merged into an existing one.
    pub merged: bool,
    pub cancelled: bool,
}

fn cancelled_err() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "cancelled")
}

pub fn fmt_eta(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s < 5 {
        ts("a few seconds").into()
    } else if s < 60 {
        format!("{} s", s)
    } else if s < 3600 {
        let m = s / 60;
        let r = s % 60;
        if m < 10 && r >= 5 { format!("{} min {} s", m, r) } else { format!("{} min", (s + 30) / 60) }
    } else {
        tf!("{} h {} min", s / 3600, (s % 3600) / 60)
    }
}

struct Progress<'a> {
    total_bytes: u64,
    done_bytes: u64,
    last: Instant,
    started: Instant,
    win_start: Instant,
    win_bytes: u64,
    speed: f64,
    report: &'a dyn Fn(f32, String),
    label: &'static str,
    cancel: &'a AtomicBool,
}

impl<'a> Progress<'a> {
    fn new(label: &'static str, total: u64, report: &'a dyn Fn(f32, String), cancel: &'a AtomicBool) -> Self {
        let now = Instant::now();
        Progress { total_bytes: total, done_bytes: 0, last: now, started: now, win_start: now, win_bytes: 0, speed: 0.0, report, label, cancel }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(AtomicOrdering::Relaxed)
    }

    fn tick(&mut self, n: u64, name: &str) {
        self.done_bytes += n;
        if self.last.elapsed().as_millis() < 120 {
            return;
        }
        self.last = Instant::now();
        let w = self.win_start.elapsed().as_secs_f64();
        if w >= 0.5 {
            let inst = (self.done_bytes - self.win_bytes) as f64 / w;
            self.speed = if self.speed <= 0.0 { inst } else { self.speed * 0.7 + inst * 0.3 };
            self.win_start = Instant::now();
            self.win_bytes = self.done_bytes;
        }
        let total = self.total_bytes.max(self.done_bytes);
        let f = if total == 0 { 0.0 } else { self.done_bytes as f32 / total as f32 };
        let mut t = tf!("{} {} · {} of {}", self.label, name, fmt_size(self.done_bytes), fmt_size(total));
        if self.speed > 1.0 && self.started.elapsed().as_millis() > 1000 {
            t.push_str(&tf!(" · {}/s · {} left", fmt_size(self.speed as u64), fmt_eta((total - self.done_bytes) as f64 / self.speed)));
        }
        (self.report)(f, t);
    }
}

fn tree_size(p: &Path) -> u64 {
    match fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => fs::read_dir(p).map(|rd| rd.flatten().map(|e| tree_size(&e.path())).sum()).unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

pub fn tree_stats(p: &Path, files: &mut u64, dirs_: &mut u64, bytes: &mut u64, cancel: &dyn Fn() -> bool) {
    if cancel() {
        return;
    }
    if let Ok(rd) = fs::read_dir(p) {
        for e in rd.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => {
                    *dirs_ += 1;
                    tree_stats(&e.path(), files, dirs_, bytes, cancel);
                }
                Ok(_) => {
                    *files += 1;
                    *bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
                }
                Err(_) => {}
            }
        }
    }
}

fn copy_tree(src: &Path, dst: &Path, pr: &mut Progress) -> io::Result<()> {
    if pr.cancelled() {
        return Err(cancelled_err());
    }
    let md = fs::symlink_metadata(src)?;
    if md.is_dir() {
        fs::create_dir_all(dst)?;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy_tree(&e.path(), &dst.join(e.file_name()), pr)?;
        }
        Ok(())
    } else {
        let name = src.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        copy_file_progress(src, dst, pr, &name)
    }
}

fn copy_file_progress(src: &Path, dst: &Path, pr: &mut Progress, name: &str) -> io::Result<()> {
    use std::io::{Read, Write};
    let mut r = fs::File::open(src)?;
    let mut w = fs::File::create(dst)?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if pr.cancelled() {
            drop(w);
            let _ = fs::remove_file(dst);
            return Err(cancelled_err());
        }
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n])?;
        pr.tick(n as u64, name);
    }
    if let Ok(md) = fs::metadata(src) {
        let _ = w.set_permissions(md.permissions());
        if let Ok(t) = md.modified() {
            let _ = w.set_modified(t);
        }
    }
    Ok(())
}

/// Moves the item at `target` to the bin so something else can take its place.
fn make_room(target: &Path) -> io::Result<()> {
    trash::delete(target).map_err(|e| io::Error::other(tf!("couldn't move the old \"{}\" to the {} ({})", file_name(target), trash_word(), e)))
}

fn remove_any(p: &Path) -> io::Result<()> {
    if fs::symlink_metadata(p)?.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) }
}

/// Copy or move one item to `target` (which doesn't exist).
fn transfer(kind: OpKind, src: &Path, target: &Path, pr: &mut Progress) -> io::Result<()> {
    if kind == OpKind::Move {
        match fs::rename(src, target) {
            Ok(_) => Ok(()),
            Err(_) => {
                // different drive: copy, then remove the original
                pr.total_bytes += tree_size(src);
                match copy_tree(src, target, pr) {
                    Ok(_) => remove_any(src),
                    Err(e) => {
                        let _ = remove_any(target);
                        Err(e)
                    }
                }
            }
        }
    } else {
        match copy_tree(src, target, pr) {
            Ok(_) => Ok(()),
            Err(e) => {
                // nothing half-copied stays behind
                if e.kind() == io::ErrorKind::Interrupted {
                    let _ = remove_any(target);
                }
                Err(e)
            }
        }
    }
}

/// Merge the folder `src` into the existing folder `dst`. Files that exist in both are replaced
/// (the old ones go to the bin).
fn merge_into(kind: OpKind, src: &Path, dst: &Path, pr: &mut Progress, res: &mut OpResult) -> io::Result<()> {
    for e in fs::read_dir(src)? {
        if pr.cancelled() {
            return Err(cancelled_err());
        }
        let e = e?;
        let s = e.path();
        let d = dst.join(e.file_name());
        let s_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        match fs::symlink_metadata(&d) {
            Ok(dm) if dm.is_dir() && s_dir => merge_into(kind, &s, &d, pr, res)?,
            Ok(_) => {
                make_room(&d)?;
                res.replaced += 1;
                transfer(kind, &s, &d, pr)?;
            }
            Err(_) => transfer(kind, &s, &d, pr)?,
        }
    }
    if kind == OpKind::Move {
        // only goes away when everything was moved out
        let _ = fs::remove_dir(src);
    }
    Ok(())
}

/// Top-level items of `sources` that already exist (by name) in `dest`.
pub fn find_conflicts(kind: OpKind, sources: &[PathBuf], dest: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    for s in sources {
        if s.parent() == Some(dest) || dest.starts_with(s) {
            continue; // duplicates in the same folder always get a new name; moving there is a no-op
        }
        let _ = kind;
        let target = dest.join(file_name(s));
        if fs::symlink_metadata(&target).is_ok() {
            out.push((s.clone(), target));
        }
    }
    out
}

pub fn run_op(
    kind: OpKind,
    sources: &[PathBuf],
    dest: Option<&Path>,
    policies: &std::collections::HashMap<PathBuf, Conflict>,
    cancel: &AtomicBool,
    report: &dyn Fn(f32, String),
) -> OpResult {
    let mut res = OpResult::default();
    let n = sources.len().max(1);
    match kind {
        OpKind::Trash => {
            for (i, s) in sources.iter().enumerate() {
                if cancel.load(AtomicOrdering::Relaxed) {
                    res.cancelled = true;
                    break;
                }
                report(i as f32 / n as f32, tf!("Moving to the {}: {} ({} of {})", trash_word(), file_name(s), i + 1, sources.len()));
                match trash::delete(s) {
                    Ok(_) => res.done += 1,
                    Err(e) => res.errors.push(format!("{}: {}", file_name(s), e)),
                }
            }
        }
        OpKind::Delete => {
            for (i, s) in sources.iter().enumerate() {
                if cancel.load(AtomicOrdering::Relaxed) {
                    res.cancelled = true;
                    break;
                }
                report(i as f32 / n as f32, tf!("Deleting {} ({} of {})", file_name(s), i + 1, sources.len()));
                match remove_any(s) {
                    Ok(_) => res.done += 1,
                    Err(e) => res.errors.push(format!("{}: {}", file_name(s), friendly_err(&e))),
                }
            }
        }
        OpKind::Copy | OpKind::Move => {
            let Some(dest) = dest else { return res };
            if kind == OpKind::Copy {
                report(0.0, tf!("Getting ready to copy {} item{}…", sources.len(), if sources.len() == 1 { "" } else { "s" }));
            }
            let total: u64 = if kind == OpKind::Copy { sources.iter().map(|s| tree_size(s)).sum() } else { 0 };
            let mut pr = Progress::new(if kind == OpKind::Copy { ts("Copying") } else { ts("Moving") }, total, report, cancel);
            for (i, s) in sources.iter().enumerate() {
                if pr.cancelled() {
                    res.cancelled = true;
                    break;
                }
                let name = file_name(s);
                // never copy/move a folder into itself
                if dest.starts_with(s) {
                    res.errors.push(tf!("{}: can't put a folder inside itself", name));
                    continue;
                }
                let is_dir = s.is_dir();
                let same_parent = s.parent() == Some(dest);
                if kind == OpKind::Move && same_parent {
                    continue;
                }
                let plain = dest.join(&name);
                let exists = !same_parent && fs::symlink_metadata(&plain).is_ok();
                let policy = if exists { policies.get(s).copied().unwrap_or(Conflict::KeepBoth) } else { Conflict::KeepBoth };
                if kind == OpKind::Move {
                    report(i as f32 / n as f32, tf!("Moving {} ({} of {})", name, i + 1, sources.len()));
                }
                let r: io::Result<Option<PathBuf>> = match policy {
                    Conflict::Skip => {
                        res.skipped += 1;
                        continue;
                    }
                    Conflict::Replace => {
                        let dst_is_dir = fs::metadata(&plain).map(|m| m.is_dir()).unwrap_or(false);
                        if is_dir && dst_is_dir {
                            res.merged = true;
                            merge_into(kind, s, &plain, &mut pr, &mut res).map(|_| None)
                        } else {
                            make_room(&plain).and_then(|_| {
                                res.replaced += 1;
                                transfer(kind, s, &plain, &mut pr).map(|_| Some(plain.clone()))
                            })
                        }
                    }
                    Conflict::KeepBoth => {
                        let target = unique_copy_name(dest, &name, is_dir);
                        transfer(kind, s, &target, &mut pr).map(|_| Some(target))
                    }
                };
                match r {
                    Ok(Some(t)) => {
                        res.done += 1;
                        res.pairs.push((s.clone(), t.clone()));
                        res.created.push(t);
                    }
                    Ok(None) => res.done += 1,
                    Err(_) if pr.cancelled() => {
                        res.cancelled = true;
                        break;
                    }
                    Err(e) => res.errors.push(format!("{}: {}", name, friendly_err(&e))),
                }
            }
        }
        OpKind::Zip => {
            if let Some(dest) = dest {
                match zip_create(sources, dest, report, cancel) {
                    Ok(_) => {
                        res.done = sources.len();
                        res.created.push(dest.to_path_buf());
                    }
                    Err(e) => {
                        let _ = fs::remove_file(dest);
                        if cancel.load(AtomicOrdering::Relaxed) {
                            res.cancelled = true;
                        } else {
                            res.errors.push(e);
                        }
                    }
                }
            }
        }
        OpKind::Extract => {
            if let (Some(dest), Some(src)) = (dest, sources.first()) {
                match extract(src, dest, report, cancel) {
                    Ok(n) => {
                        res.done = n;
                        res.created.push(dest.to_path_buf());
                    }
                    Err(_) if cancel.load(AtomicOrdering::Relaxed) => {
                        res.cancelled = true;
                        res.created.push(dest.to_path_buf());
                    }
                    Err(e) => res.errors.push(format!("{}: {}", file_name(src), e)),
                }
            }
        }
    }
    res
}

/// Undo of moves: put every `to` back at its `from`.
pub fn move_back(pairs: &[(PathBuf, PathBuf)]) -> OpResult {
    let mut res = OpResult::default();
    let noop = |_: f32, _: String| {};
    let never = AtomicBool::new(false);
    for (from, to) in pairs {
        if from.exists() {
            res.errors.push(tf!("{}: something with that name is already back there", file_name(from)));
            continue;
        }
        let r = fs::rename(to, from).or_else(|_| {
            let mut pr = Progress::new("", 0, &noop, &never);
            copy_tree(to, from, &mut pr).and_then(|_| remove_any(to))
        });
        match r {
            Ok(_) => {
                res.done += 1;
                res.created.push(from.clone());
            }
            Err(e) => res.errors.push(format!("{}: {}", file_name(to), friendly_err(&e))),
        }
    }
    res
}

pub fn file_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string())
}

pub fn trash_word() -> &'static str {
    if cfg!(windows) { ts("Recycle Bin") } else { ts("Trash") }
}

// ───────────── search ─────────────

/// Walks `root` and calls `found` with batches of matches. Stops when `cancel()` returns true.
pub fn search(root: &Path, query: &str, show_hidden: bool, cancel: &dyn Fn() -> bool, found: &mut dyn FnMut(Vec<Entry>, usize)) {
    let q = query.to_lowercase();
    let mut stack = vec![root.to_path_buf()];
    let mut batch = Vec::new();
    let mut scanned = 0usize;
    let mut total = 0usize;
    let mut last = Instant::now();
    while let Some(dir) = stack.pop() {
        if cancel() {
            return;
        }
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        let loc = dir.display().to_string();
        for de in rd.flatten() {
            scanned += 1;
            let name = de.file_name().to_string_lossy().into_owned();
            let Ok(ft) = de.file_type() else { continue };
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            if ft.is_dir() && !ft.is_symlink() {
                stack.push(de.path());
            }
            if name.to_lowercase().contains(&q) {
                if let Ok(md) = de.metadata() {
                    let e = entry_from(de.path(), name, &md, loc.clone());
                    if !show_hidden && e.hidden {
                        continue;
                    }
                    batch.push(e);
                    total += 1;
                }
            }
        }
        if !batch.is_empty() && (batch.len() >= 400 || last.elapsed().as_millis() > 150) {
            found(std::mem::take(&mut batch), scanned);
            last = Instant::now();
        }
        if total >= 20000 {
            break;
        }
    }
    found(batch, scanned);
}

// ───────────── launching ─────────────

pub fn open_file(p: &Path) -> Result<(), String> {
    open::that_detached(p).map_err(|e| e.to_string())
}

pub fn open_with(p: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        std::process::Command::new("rundll32.exe")
            .arg(format!("shell32.dll,OpenAs_RunDLL {}", p.display()))
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        open_file(p)
    }
}

pub fn open_terminal(dir: &Path) -> Result<(), String> {
    use std::process::Command;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x10;
        if Command::new("wt.exe").arg("-d").arg(dir).spawn().is_ok() {
            return Ok(());
        }
        return Command::new("powershell.exe")
            .arg("-NoExit")
            .current_dir(dir)
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string());
    }
    #[cfg(target_os = "macos")]
    {
        return Command::new("open").args(["-a", "Terminal"]).arg(dir).spawn().map(|_| ()).map_err(|e| e.to_string());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let d = dir.to_string_lossy().to_string();
        let tries: Vec<(&str, Vec<String>)> = vec![
            ("konsole", vec!["--workdir".into(), d.clone()]),
            ("ptyxis", vec!["--working-directory".into(), d.clone()]),
            ("kgx", vec!["--working-directory".into(), d.clone()]),
            ("gnome-terminal", vec![format!("--working-directory={}", d)]),
            ("xfce4-terminal", vec![format!("--working-directory={}", d)]),
            ("alacritty", vec!["--working-directory".into(), d.clone()]),
            ("kitty", vec!["--directory".into(), d.clone()]),
            ("x-terminal-emulator", vec![]),
            ("xterm", vec![]),
        ];
        for (bin, args) in tries {
            if Command::new(bin).args(&args).current_dir(dir).spawn().is_ok() {
                return Ok(());
            }
        }
        Err(ts("No terminal app found").into())
    }
}

pub fn sha256_file(p: &Path, cancel: &dyn Fn() -> bool) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = fs::File::open(p).map_err(|e| friendly_err(&e))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cancel() {
            return Err("cancelled".into());
        }
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{:02x}", b)).collect())
}


pub fn expand_user(s: &str) -> PathBuf {
    let s = s.trim().trim_matches('"');
    if s == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(s));
    }
    if let Some(rest) = s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")) {
        if let Some(h) = dirs::home_dir() {
            return h.join(rest);
        }
    }
    #[cfg(windows)]
    {
        // "C:" → "C:\"
        if s.len() == 2 && s.ends_with(':') {
            return PathBuf::from(format!("{}\\", s));
        }
        // %USERPROFILE% style variables
        if s.contains('%') {
            let mut out = String::new();
            let mut parts = s.split('%');
            if let Some(first) = parts.next() {
                out.push_str(first);
            }
            let rest: Vec<&str> = parts.collect();
            let mut i = 0;
            while i < rest.len() {
                if i + 1 < rest.len() + 1 {
                    let var = rest[i];
                    match std::env::var(var) {
                        Ok(v) if i + 1 <= rest.len() => {
                            out.push_str(&v);
                            if i + 1 < rest.len() {
                                out.push_str(rest[i + 1]);
                            }
                        }
                        _ => {
                            out.push('%');
                            out.push_str(var);
                            out.push('%');
                            if i + 1 < rest.len() {
                                out.push_str(rest[i + 1]);
                            }
                        }
                    }
                }
                i += 2;
            }
            return PathBuf::from(out);
        }
    }
    PathBuf::from(s)
}

// ───────────── hashing ─────────────

pub const HASH_ALGOS: [&str; 6] = ["CRC32", "MD5", "SHA-1", "SHA-256", "SHA-512", "BLAKE3"];

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

enum Hasher {
    Crc(crc32fast::Hasher),
    Md5(md5::Md5),
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
    Blake3(Box<blake3::Hasher>),
}

impl Hasher {
    fn new(algo: &str) -> Option<Hasher> {
        use sha2::Digest;
        Some(match algo {
            "CRC32" => Hasher::Crc(crc32fast::Hasher::new()),
            "MD5" => Hasher::Md5(md5::Md5::new()),
            "SHA-1" => Hasher::Sha1(sha1::Sha1::new()),
            "SHA-256" => Hasher::Sha256(sha2::Sha256::new()),
            "SHA-512" => Hasher::Sha512(sha2::Sha512::new()),
            "BLAKE3" => Hasher::Blake3(Box::new(blake3::Hasher::new())),
            _ => return None,
        })
    }
    fn update(&mut self, d: &[u8]) {
        use sha2::Digest;
        match self {
            Hasher::Crc(h) => h.update(d),
            Hasher::Md5(h) => h.update(d),
            Hasher::Sha1(h) => h.update(d),
            Hasher::Sha256(h) => h.update(d),
            Hasher::Sha512(h) => h.update(d),
            Hasher::Blake3(h) => {
                h.update(d);
            }
        }
    }
    fn finish(self) -> String {
        use sha2::Digest;
        match self {
            Hasher::Crc(h) => format!("{:08x}", h.finalize()),
            Hasher::Md5(h) => hex(&h.finalize()),
            Hasher::Sha1(h) => hex(&h.finalize()),
            Hasher::Sha256(h) => hex(&h.finalize()),
            Hasher::Sha512(h) => hex(&h.finalize()),
            Hasher::Blake3(h) => h.finalize().to_hex().to_string(),
        }
    }
}

/// Reads the file once and feeds every algorithm in parallel (one thread each).
pub fn hash_file(p: &Path, algos: &[&str], progress: &dyn Fn(f32), cancel: &dyn Fn() -> bool) -> Result<Vec<(String, String)>, String> {
    use std::io::Read;
    use std::sync::mpsc::sync_channel;
    use std::sync::Arc;
    let mut f = fs::File::open(p).map_err(|e| friendly_err(&e))?;
    let total = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut senders = Vec::new();
    let mut handles = Vec::new();
    for a in algos {
        let Some(mut h) = Hasher::new(a) else { continue };
        let (tx, rx) = sync_channel::<Option<Arc<Vec<u8>>>>(6);
        senders.push(tx);
        let name = a.to_string();
        handles.push(std::thread::spawn(move || {
            while let Ok(Some(chunk)) = rx.recv() {
                h.update(&chunk);
            }
            (name, h.finish())
        }));
    }
    let mut done: u64 = 0;
    let mut last = Instant::now();
    loop {
        if cancel() {
            for s in &senders {
                let _ = s.send(None);
            }
            return Err("cancelled".into());
        }
        let mut buf = vec![0u8; 1 << 20];
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        buf.truncate(n);
        let chunk = Arc::new(buf);
        for s in &senders {
            let _ = s.send(Some(chunk.clone()));
        }
        done += n as u64;
        if last.elapsed().as_millis() > 100 {
            last = Instant::now();
            progress(if total > 0 { done as f32 / total as f32 } else { 0.0 });
        }
    }
    for s in &senders {
        let _ = s.send(None);
    }
    let mut out = Vec::new();
    for h in handles {
        if let Ok(r) = h.join() {
            out.push(r);
        }
    }
    // keep the requested order
    out.sort_by_key(|(n, _)| algos.iter().position(|a| a == n).unwrap_or(99));
    Ok(out)
}

pub fn algo_for_hash(h: &str) -> Option<&'static str> {
    let h = h.trim();
    if !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match h.len() {
        8 => Some("CRC32"),
        32 => Some("MD5"),
        40 => Some("SHA-1"),
        64 => Some("SHA-256"),
        128 => Some("SHA-512"),
        _ => None,
    }
}

pub fn is_checksum_file(name: &str) -> bool {
    let l = name.to_lowercase();
    l.ends_with(".sha256") || l.ends_with(".sha512") || l.ends_with(".sha1") || l.ends_with(".md5") || l.contains("sums") || l.ends_with(".sha256sum")
}

/// Parses "hash  name", "hash *name" and BSD "SHA256 (name) = hash" lines.
pub fn parse_checksums(text: &str) -> Vec<(String, String)> {
    let mut v = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if let (Some(open), Some(eq)) = (l.find(" ("), l.rfind(") = ")) {
            if open < eq {
                v.push((l[eq + 4..].trim().to_lowercase(), l[open + 2..eq].to_string()));
                continue;
            }
        }
        if let Some((h, rest)) = l.split_once(char::is_whitespace) {
            let name = rest.trim_start().trim_start_matches('*').to_string();
            if algo_for_hash(h).is_some() && !name.is_empty() {
                v.push((h.to_lowercase(), name));
            }
        }
    }
    v
}

// ───────────── archives ─────────────

pub fn archive_kind(name: &str) -> Option<&'static str> {
    let l = name.to_lowercase();
    if l.ends_with(".zip") { Some("zip") }
    else if l.ends_with(".tar.gz") || l.ends_with(".tgz") { Some("tgz") }
    else if l.ends_with(".tar") { Some("tar") }
    else if l.ends_with(".gz") { Some("gz") }
    else { None }
}

pub fn archive_stem(name: &str) -> String {
    let l = name.to_lowercase();
    for suf in [".tar.gz", ".tgz", ".zip", ".tar", ".gz"] {
        if l.ends_with(suf) {
            return name[..name.len() - suf.len()].to_string();
        }
    }
    name.to_string()
}

pub fn zip_create(srcs: &[PathBuf], dest: &Path, report: &dyn Fn(f32, String), cancel: &AtomicBool) -> Result<(), String> {
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;
    let base = srcs.first().and_then(|p| p.parent()).map(|p| p.to_path_buf()).unwrap_or_default();
    let total: u64 = srcs.iter().map(|s| tree_size(s)).sum();
    let file = fs::File::create(dest).map_err(|e| friendly_err(&e))?;
    let mut zw = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true);
    let mut done = 0u64;
    let mut last = Instant::now();
    let mut stack: Vec<PathBuf> = srcs.to_vec();
    let mut buf = vec![0u8; 1 << 20];
    while let Some(p) = stack.pop() {
        let rel = p.strip_prefix(&base).unwrap_or(&p).to_string_lossy().replace('\\', "/");
        let md = fs::symlink_metadata(&p).map_err(|e| friendly_err(&e))?;
        if md.is_dir() {
            zw.add_directory(format!("{}/", rel), opts).map_err(|e| e.to_string())?;
            if let Ok(rd) = fs::read_dir(&p) {
                for e in rd.flatten() {
                    stack.push(e.path());
                }
            }
        } else if md.is_file() {
            // keep the file's real modification time inside the zip
            let mut o = opts;
            if let Ok(t) = md.modified() {
                use chrono::{Datelike, Timelike};
                let dt: chrono::DateTime<chrono::Local> = t.into();
                if let Ok(zt) = zip::DateTime::from_date_and_time(dt.year().clamp(1980, 2107) as u16, dt.month() as u8, dt.day() as u8, dt.hour() as u8, dt.minute() as u8, dt.second() as u8) {
                    o = o.last_modified_time(zt);
                }
            }
            zw.start_file(rel.clone(), o).map_err(|e| e.to_string())?;
            let mut f = fs::File::open(&p).map_err(|e| friendly_err(&e))?;
            loop {
                if cancel.load(AtomicOrdering::Relaxed) {
                    return Err(ts("Cancelled").into());
                }
                let n = f.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                zw.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                done += n as u64;
                if last.elapsed().as_millis() > 80 {
                    last = Instant::now();
                    report(if total > 0 { done as f32 / total as f32 } else { 0.0 }, tf!("Compressing {} · {} of {}", rel, fmt_size(done), fmt_size(total)));
                }
            }
        }
    }
    zw.finish().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn extract(archive: &Path, dest: &Path, report: &dyn Fn(f32, String), cancel: &AtomicBool) -> Result<usize, String> {
    let name = file_name(archive);
    let kind = archive_kind(&name).ok_or(ts("Not an archive SKJ Files can open yet"))?;
    fs::create_dir_all(dest).map_err(|e| friendly_err(&e))?;
    let mut count = 0usize;
    match kind {
        "zip" => {
            let f = fs::File::open(archive).map_err(|e| friendly_err(&e))?;
            let mut za = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|e| e.to_string())?;
            let n = za.len();
            for i in 0..n {
                if cancel.load(AtomicOrdering::Relaxed) {
                    return Err(ts("Cancelled").into());
                }
                let mut zf = za.by_index(i).map_err(|e| e.to_string())?;
                let Some(rel) = zf.enclosed_name() else { continue };
                let mut out = dest.join(rel);
                if zf.is_dir() {
                    fs::create_dir_all(&out).map_err(|e| friendly_err(&e))?;
                } else {
                    if let Some(parent) = out.parent() {
                        fs::create_dir_all(parent).map_err(|e| friendly_err(&e))?;
                        // never overwrite: an existing file gets " - Copy" instead
                        if out.exists() {
                            out = unique_copy_name(parent, &file_name(&out), false);
                        }
                    }
                    let mut w = fs::File::create(&out).map_err(|e| friendly_err(&e))?;
                    std::io::copy(&mut zf, &mut w).map_err(|e| e.to_string())?;
                    count += 1;
                }
                if i % 20 == 0 {
                    report(i as f32 / n.max(1) as f32, tf!("Extracting {} · {} of {}", name, i + 1, n));
                }
            }
        }
        "tar" | "tgz" => {
            let f = fs::File::open(archive).map_err(|e| friendly_err(&e))?;
            let reader: Box<dyn std::io::Read> = if kind == "tgz" { Box::new(flate2::read::GzDecoder::new(std::io::BufReader::new(f))) } else { Box::new(std::io::BufReader::new(f)) };
            let mut ar = tar::Archive::new(reader);
            for e in ar.entries().map_err(|e| e.to_string())? {
                if cancel.load(AtomicOrdering::Relaxed) {
                    return Err(ts("Cancelled").into());
                }
                let mut e = e.map_err(|e| e.to_string())?;
                // never overwrite existing files
                if let Ok(p) = e.path() {
                    let target = dest.join(&p);
                    if target.is_file() {
                        continue;
                    }
                }
                // unpack_in refuses paths that escape `dest`
                if e.unpack_in(dest).map_err(|e| e.to_string())? {
                    count += 1;
                }
                if count % 50 == 0 {
                    report(0.5, tf!("Extracting {} · {} items", name, count));
                }
            }
        }
        "gz" => {
            let f = fs::File::open(archive).map_err(|e| friendly_err(&e))?;
            let mut d = flate2::read::GzDecoder::new(std::io::BufReader::new(f));
            let out = unique_copy_name(dest, &archive_stem(&name), false);
            let mut w = fs::File::create(&out).map_err(|e| friendly_err(&e))?;
            std::io::copy(&mut d, &mut w).map_err(|e| e.to_string())?;
            count = 1;
        }
        _ => {}
    }
    Ok(count)
}

// ───────────── git / editors / trash ─────────────

pub fn git_branch(dir: &Path) -> Option<String> {
    for a in dir.ancestors() {
        let g = a.join(".git");
        let gitdir = if g.is_dir() {
            g
        } else if g.is_file() {
            let t = fs::read_to_string(&g).ok()?;
            let p = t.trim().strip_prefix("gitdir:")?.trim().to_string();
            let p = PathBuf::from(p);
            if p.is_absolute() { p } else { a.join(p) }
        } else {
            continue;
        };
        let head = fs::read_to_string(gitdir.join("HEAD")).ok()?;
        let head = head.trim();
        return Some(match head.strip_prefix("ref: refs/heads/") {
            Some(b) => b.to_string(),
            None => head.chars().take(7).collect(),
        });
    }
    None
}

pub fn open_vscode(p: &Path) -> Result<(), String> {
    use std::process::Command;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        return Command::new("cmd.exe")
            .args(["/C", "code"])
            .arg(p)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
            .map_err(|_| ts("VS Code (the `code` command) wasn't found").into());
    }
    #[cfg(not(windows))]
    {
        for bin in ["code", "codium", "code-oss"] {
            if Command::new(bin).arg(p).spawn().is_ok() {
                return Ok(());
            }
        }
        Err(ts("VS Code (the `code` command) wasn't found").into())
    }
}

/// Puts the given (previously trashed) paths back where they came from.
pub fn restore_from_trash(paths: &[PathBuf]) -> Result<usize, String> {
    #[cfg(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))))]
    {
        let items = trash::os_limited::list().map_err(|e| e.to_string())?;
        let mut picked: Vec<trash::TrashItem> = Vec::new();
        for p in paths {
            let best = items
                .iter()
                .filter(|it| it.original_parent.join(&it.name) == *p)
                .max_by_key(|it| it.time_deleted)
                .cloned();
            if let Some(b) = best {
                picked.push(b);
            }
        }
        let n = picked.len();
        if n == 0 {
            return Err(ts("Couldn't find those items in the bin anymore").into());
        }
        trash::os_limited::restore_all(picked).map_err(|e| e.to_string())?;
        Ok(n)
    }
    #[cfg(not(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android")))))]
    {
        let _ = paths;
        Err(ts("Restoring from the bin isn't supported on this system yet").into())
    }
}

/// Batch rename: find/replace, then an optional pattern with {name} {n} {nn} {nnn} {ext} {date}.
pub fn batch_name(e: &Entry, find: &str, replace: &str, pattern: &str, n: usize) -> String {
    let (stem, ext) = split_name(&e.name, e.is_dir);
    let stem = if find.is_empty() { stem } else { stem.replace(find, replace) };
    if pattern.trim().is_empty() {
        return format!("{}{}", stem, ext);
    }
    let date = e.modified.map(|t| {
        let dt: chrono::DateTime<chrono::Local> = t.into();
        dt.format("%Y-%m-%d").to_string()
    }).unwrap_or_default();
    let ext_plain = ext.trim_start_matches('.');
    let mut out = pattern
        .replace("{name}", &stem)
        .replace("{nnn}", &format!("{:03}", n))
        .replace("{nn}", &format!("{:02}", n))
        .replace("{n}", &n.to_string())
        .replace("{date}", &date)
        .replace("{ext}", ext_plain);
    if !pattern.contains("{ext}") && !ext.is_empty() {
        out.push_str(&ext);
    }
    out
}

pub fn friendly_time(t: Option<SystemTime>) -> String {
    let Some(t) = t else { return String::new() };
    let dt: chrono::DateTime<chrono::Local> = t.into();
    let now = chrono::Local::now();
    let d = now.date_naive().signed_duration_since(dt.date_naive()).num_days();
    match d {
        0 => tf!("Today {}", dt.format("%H:%M")),
        1 => tf!("Yesterday {}", dt.format("%H:%M")),
        2..=6 => format!("{} {}", i18n::weekday(chrono::Datelike::weekday(&dt)), dt.format("%H:%M")),
        _ => fmt_time(Some(t)),
    }
}


// ───────────── Recycle Bin / Trash view ─────────────

/// Whether this system lets SKJ Files list and restore what's in the bin.
pub const BIN_VIEW: bool = cfg!(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))));

#[cfg(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))))]
mod bin {
    use super::*;
    pub type Item = trash::TrashItem;

    /// Everything in the bin, as entries (path = the bin's own id, location = original folder,
    /// modified = when it was deleted).
    pub fn list() -> Result<Vec<(Entry, Item)>, String> {
        let items = trash::os_limited::list().map_err(|e| {
            let detail = match &e {
                trash::Error::Os { description, .. } => description.clone(),
                other => other.to_string(),
            };
            tf!("Couldn't open the {} ({})", trash_word(), detail)
        })?;
        let mut out = Vec::with_capacity(items.len());
        for it in items {
            let (is_dir, size) = match trash::os_limited::metadata(&it) {
                Ok(m) => match m.size {
                    trash::TrashItemSize::Bytes(b) => (false, b),
                    trash::TrashItemSize::Entries(_) => (true, 0),
                },
                Err(_) => (false, 0),
            };
            let name = it.name.to_string_lossy().to_string();
            let ext = if is_dir { String::new() } else { ext_of(&name) };
            let deleted = if it.time_deleted > 0 { Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(it.time_deleted as u64)) } else { None };
            let e = Entry {
                path: PathBuf::from(&it.id),
                kind: if is_dir { K_FOLDER } else { kind_for_ext(&ext) },
                ext,
                name,
                is_dir,
                size,
                modified: deleted,
                created: None,
                hidden: false,
                location: it.original_parent.display().to_string(),
            };
            out.push((e, it));
        }
        Ok(out)
    }

    /// Put items back. Items whose original place is taken are left in the bin and reported.
    pub fn restore(items: Vec<Item>) -> (usize, Vec<String>) {
        let total = items.len();
        let mut errs = Vec::new();
        let mut todo = items;
        // two deleted items with the same original path: restore only the newest
        todo.sort_by_key(|i| std::cmp::Reverse(i.time_deleted));
        let mut seen = std::collections::HashSet::new();
        todo.retain(|i| {
            let ok = seen.insert(i.original_path());
            if !ok {
                errs.push(tf!("an older \"{}\" stays in the bin", i.name.to_string_lossy()));
            }
            ok
        });
        while !todo.is_empty() {
            match trash::os_limited::restore_all(todo.clone()) {
                Ok(()) => break,
                Err(trash::Error::RestoreCollision { path, remaining_items }) => {
                    errs.push(tf!("\"{}\" already exists in its original folder", file_name(&path)));
                    todo = remaining_items.into_iter().filter(|e| e.original_path() != path).collect();
                }
                Err(e) => {
                    errs.push(e.to_string());
                    break;
                }
            }
        }
        (total.saturating_sub(errs.len()), errs)
    }

    pub fn purge(items: Vec<Item>) -> Result<usize, String> {
        let n = items.len();
        trash::os_limited::purge_all(items).map_err(|e| e.to_string())?;
        Ok(n)
    }

    pub fn empty() -> Result<usize, String> {
        let items = trash::os_limited::list().map_err(|e| e.to_string())?;
        purge(items)
    }
}

#[cfg(not(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android")))))]
mod bin {
    use super::*;
    #[derive(Clone, Debug)]
    pub struct Item;
    pub fn list() -> Result<Vec<(Entry, Item)>, String> {
        Err(ts("The bin can't be listed on this system").into())
    }
    pub fn restore(_items: Vec<Item>) -> (usize, Vec<String>) {
        (0, vec![ts("Not supported on this system").into()])
    }
    pub fn purge(_items: Vec<Item>) -> Result<usize, String> {
        Err(ts("Not supported on this system").into())
    }
    pub fn empty() -> Result<usize, String> {
        Err(ts("Not supported on this system").into())
    }
}

pub use bin::{empty as bin_empty, list as bin_list, purge as bin_purge, restore as bin_restore, Item as BinItem};


// ───────────── network places ─────────────

/// What the user typed in "Connect to server", in the form this OS opens.
/// Windows: `\\server\share` (smb:// is converted). Others: a URL for gio / the Finder.
pub fn normalize_server(input: &str) -> Option<String> {
    let t = input.trim();
    if t.is_empty() {
        return None;
    }
    if cfg!(windows) {
        if let Some(rest) = t.strip_prefix("smb://").or_else(|| t.strip_prefix("\\\\")).or_else(|| t.strip_prefix("//")) {
            let rest = rest.trim_end_matches(['/', '\\']);
            return Some(format!("\\\\{}", rest.replace('/', "\\")));
        }
        return None;
    }
    if t.contains("://") {
        return Some(t.trim_end_matches('/').to_string());
    }
    if let Some(rest) = t.strip_prefix("\\\\").or_else(|| t.strip_prefix("//")) {
        return Some(format!("smb://{}", rest.replace('\\', "/").trim_end_matches('/')));
    }
    // a bare host name
    Some(format!("smb://{}", t))
}

/// Label for a saved server: "photos on nas"
pub fn server_label(s: &str) -> String {
    let body = s.split_once("://").map(|x| x.1).unwrap_or(s).trim_start_matches(['\\', '/']);
    let parts: Vec<&str> = body.split(['/', '\\']).filter(|x| !x.is_empty()).collect();
    match parts.len() {
        0 => s.to_string(),
        1 => parts[0].to_string(),
        _ => tf!("{} on {}", parts[parts.len() - 1], parts[0].rsplit('@').next().unwrap_or(parts[0])),
    }
}

/// Connect to a server and return the folder to open. Runs on a worker thread (can take a while).
pub fn connect_server(addr: &str, user: &str, password: &str) -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let p = PathBuf::from(addr);
        if !user.is_empty() {
            // sign in for this session (like "Connect using different credentials")
            let out = std::process::Command::new("net")
                .args(["use", addr])
                .arg(password)
                .arg(format!("/user:{}", user))
                .creation_flags(0x0800_0000)
                .output()
                .map_err(|e| e.to_string())?;
            if !out.status.success() {
                let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                return Err(if msg.is_empty() { ts("Windows refused the sign-in").into() } else { msg });
            }
        }
        match fs::read_dir(&p) {
            Ok(_) => Ok(p),
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => Err(ts("This share needs a user name and password").into()),
            Err(e) => Err(friendly_err(&e)),
        }
    }
    #[cfg(target_os = "macos")]
    {
        let url = if user.is_empty() {
            addr.to_string()
        } else {
            let (scheme, rest) = addr.split_once("://").unwrap_or(("smb", addr));
            format!("{}://{}:{}@{}", scheme, user, password, rest)
        };
        let script = format!("mount volume \"{}\"", url.replace('"', ""));
        let out = std::process::Command::new("osascript").args(["-e", &script]).output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        // "file nas:photos:" → /Volumes/photos
        let share = addr.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string();
        let p = PathBuf::from("/Volumes").join(&share);
        if p.is_dir() { Ok(p) } else { Err(ts("Connected, but the share didn't appear in /Volumes").into()) }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut child = Command::new("gio")
            .args(["mount", addr])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| ts("Connecting needs the `gio` tool (package glib2 / gvfs)").to_string())?;
        if let Some(mut si) = child.stdin.take() {
            // answers for gio's questions: user, domain, password
            let _ = writeln!(si, "{}\n\n{}", if user.is_empty() { "guest" } else { user }, password);
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        let err = String::from_utf8_lossy(&out.stderr).to_string();
        if !out.status.success() && !err.contains("already mounted") {
            if err.contains("implement mount") || err.contains("not supported") {
                return Err(ts("this system can't open network shares yet — install gvfs-smb (Fedora: sudo dnf install gvfs-smb)").into());
            }
            let e = err.lines().last().unwrap_or("").trim().to_string();
            return Err(if e.is_empty() { ts("Couldn't connect").into() } else { e });
        }
        let info = Command::new("gio").args(["info", addr]).output().map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&info.stdout);
        for line in text.lines() {
            if let Some(p) = line.trim().strip_prefix("local path:") {
                return Ok(PathBuf::from(p.trim()));
            }
        }
        Err(ts("Connected, but couldn't find the share's folder").into())
    }
}

// ───────────── recent files ─────────────

/// Files the system remembers as recently used: (path, time). Newest first.
pub fn system_recent_files(limit: usize) -> Vec<(PathBuf, SystemTime)> {
    let mut out: Vec<(PathBuf, SystemTime)> = Vec::new();
    #[cfg(windows)]
    {
        // %APPDATA%\Microsoft\Windows\Recent\*.lnk — opening the shortcut opens the file
        if let Some(d) = dirs::data_dir().map(|d| d.join("Microsoft").join("Windows").join("Recent")) {
            if let Ok(rd) = fs::read_dir(d) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().map(|x| x.eq_ignore_ascii_case("lnk")).unwrap_or(false) {
                        if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                            out.push((p, t));
                        }
                    }
                }
            }
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(f) = dirs::data_dir().map(|d| d.join("recently-used.xbel")) {
            if let Ok(text) = fs::read_to_string(f) {
                for chunk in text.split("<bookmark ").skip(1) {
                    let attr = |name: &str| -> Option<String> {
                        let k = format!("{}=\"", name);
                        let i = chunk.find(&k)? + k.len();
                        let j = chunk[i..].find('"')? + i;
                        Some(chunk[i..j].to_string())
                    };
                    let Some(href) = attr("href") else { continue };
                    let Some(path) = href.strip_prefix("file://") else { continue };
                    let p = PathBuf::from(percent_decode(path));
                    let t = attr("modified")
                        .and_then(|m| chrono::DateTime::parse_from_rfc3339(&m).ok())
                        .map(|d| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(d.timestamp().max(0) as u64))
                        .unwrap_or(SystemTime::UNIX_EPOCH);
                    if p.is_file() {
                        out.push((p, t));
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out.truncate(limit);
    out
}

#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Find files whose contents are identical, under `root`.
///
/// Size first, hash second: files with a unique size cannot have a twin, so only same-size groups
/// are ever read. That keeps a scan of a big folder tree cheap - most files never get opened.
/// Results arrive grouped, so the duplicates of one file stay together in the list.
pub fn find_duplicates(
    root: &Path,
    show_hidden: bool,
    cancel: &dyn Fn() -> bool,
    found: &mut dyn FnMut(Vec<Entry>, usize),
) {
    use std::collections::HashMap;

    // 1. walk, grouping by size
    let mut by_size: HashMap<u64, Vec<Entry>> = HashMap::new();
    let mut stack = vec![root.to_path_buf()];
    let mut scanned = 0usize;
    while let Some(dir) = stack.pop() {
        if cancel() {
            return;
        }
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        let loc = dir.display().to_string();
        for de in rd.flatten() {
            scanned += 1;
            let name = de.file_name().to_string_lossy().into_owned();
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            let Ok(ft) = de.file_type() else { continue };
            if ft.is_dir() && !ft.is_symlink() {
                stack.push(de.path());
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            let Ok(md) = de.metadata() else { continue };
            let e = entry_from(de.path(), name, &md, loc.clone());
            if !show_hidden && e.hidden {
                continue;
            }
            // empty files are all "identical"; that is noise, not a finding
            if e.size > 0 {
                by_size.entry(e.size).or_default().push(e);
            }
        }
    }

    // 2. hash only the sizes that appear more than once
    let mut groups: Vec<(u64, Vec<Entry>)> = by_size.into_iter().filter(|(_, v)| v.len() > 1).collect();
    // biggest first: the duplicates worth deleting are usually the big ones
    groups.sort_by(|a, b| b.0.cmp(&a.0));

    for (_, candidates) in groups {
        if cancel() {
            return;
        }
        let mut by_hash: HashMap<String, Vec<Entry>> = HashMap::new();
        for e in candidates {
            if cancel() {
                return;
            }
            if let Ok(h) = sha256_file(&e.path, cancel) {
                by_hash.entry(h).or_default().push(e);
            }
        }
        let mut batch: Vec<Entry> = Vec::new();
        for (_, mut same) in by_hash {
            if same.len() > 1 {
                same.sort_by(|a, b| natural_cmp(&a.name, &b.name));
                batch.append(&mut same);
            }
        }
        if !batch.is_empty() {
            found(batch, scanned);
        }
    }
    found(Vec::new(), scanned);
}

/// What is taking up the space directly under `root`: every immediate child with the full size of
/// its tree, biggest first. The sizes come back in one batch because a folder is only useful once
/// its whole tree has been measured.
pub fn disk_usage(root: &Path, show_hidden: bool, cancel: &dyn Fn() -> bool) -> Vec<Entry> {
    let Ok(rd) = fs::read_dir(root) else { return Vec::new() };
    let loc = root.display().to_string();
    let mut out: Vec<Entry> = Vec::new();
    for de in rd.flatten() {
        if cancel() {
            return Vec::new();
        }
        let name = de.file_name().to_string_lossy().into_owned();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        let Ok(md) = de.metadata() else { continue };
        let mut e = entry_from(de.path(), name, &md, loc.clone());
        if !show_hidden && e.hidden {
            continue;
        }
        if e.is_dir {
            let (mut f, mut d, mut b) = (0u64, 0u64, 0u64);
            tree_stats(&e.path, &mut f, &mut d, &mut b, cancel);
            // the real weight of the folder, so it can be sorted against plain files
            e.size = b;
        }
        out.push(e);
    }
    out.sort_by(|a, b| b.size.cmp(&a.size));
    out
}
