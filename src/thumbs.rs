//! Thumbnails. Common image formats are decoded in-process; videos, PDFs, documents, app icons and
//! other formats come from the operating system:
//! * Windows — the shell (`IShellItemImageFactory`), exactly what Explorer shows, incl. .exe icons
//! * Linux — the shared thumbnail cache other file managers fill (~/.cache/thumbnails), then
//!   `ffmpegthumbnailer`/`ffmpeg` for videos and `pdftoppm` for PDFs when they're installed
//! * macOS — Quick Look (`qlmanage`)
//!
//! A few worker threads take jobs from one queue; a new request replaces the queue, so the list
//! that's on screen now is always served first.

use crate::fsx::Entry;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Src {
    None,
    /// decode with the `image` crate
    Decode,
    /// ask the OS (thumbnail, or for apps/shortcuts the real icon)
    Os,
}

pub struct Thumb {
    pub rgba: Vec<u8>,
    pub w: u32,
    pub h: u32,
}

pub fn source_for(e: &Entry) -> Src {
    if e.is_dir {
        return Src::None;
    }
    let x = e.ext.as_str();
    match x {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "jfif" if e.size < 40 * 1024 * 1024 => Src::Decode,
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "webm" | "flv" | "m4v" | "mpg" | "mpeg" | "3gp" | "m2ts" | "mts" | "pdf" | "heic" | "heif"
        | "avif" | "tif" | "tiff" | "psd" | "svg" | "cr2" | "nef" | "arw" | "dng" => Src::Os,
        "docx" | "pptx" | "xlsx" | "odt" | "ods" | "odp" | "epub" if !cfg!(all(unix, not(target_os = "macos"))) => Src::Os,
        "exe" | "msi" | "lnk" | "url" | "ttf" | "otf" | "appx" | "msix" | "cpl" | "scr" if cfg!(windows) => Src::Os,
        _ => Src::None,
    }
}

/// Kinds whose real icon is worth showing even in the details list (small).
pub fn is_icon_kind(e: &Entry) -> bool {
    cfg!(windows) && matches!(e.ext.as_str(), "exe" | "msi" | "lnk" | "url" | "appx" | "msix" | "cpl" | "scr")
}

pub fn make(path: &Path, src: Src, size: u32) -> Option<Thumb> {
    match src {
        Src::None => None,
        Src::Decode => {
            let img = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
            let t = img.thumbnail(size, size).to_rgba8();
            let (w, h) = (t.width(), t.height());
            Some(Thumb { rgba: t.into_raw(), w, h })
        }
        Src::Os => os_thumb(path, size),
    }
}

// ───────────── worker pool ─────────────

pub struct Job {
    pub path: PathBuf,
    pub src: Src,
    pub size: u32,
}

struct Queue {
    gen: u64,
    jobs: VecDeque<Job>,
}

pub struct Pool {
    q: Arc<(Mutex<Queue>, Condvar)>,
}

impl Pool {
    /// `deliver` runs on a worker thread for every finished thumbnail (with the request's gen).
    pub fn new(workers: usize, deliver: impl Fn(u64, PathBuf, u32, Thumb) + Send + Sync + 'static) -> Pool {
        let q = Arc::new((Mutex::new(Queue { gen: 0, jobs: VecDeque::new() }), Condvar::new()));
        let deliver = Arc::new(deliver);
        for _ in 0..workers.max(1) {
            let q = q.clone();
            let deliver = deliver.clone();
            std::thread::spawn(move || {
                #[cfg(windows)]
                win::init_com();
                loop {
                    let (gen, job) = {
                        let (m, cv) = &*q;
                        let mut g = m.lock().unwrap();
                        while g.jobs.is_empty() {
                            g = cv.wait(g).unwrap();
                        }
                        let job = g.jobs.pop_front().unwrap();
                        (g.gen, job)
                    };
                    if let Some(t) = make(&job.path, job.src, job.size) {
                        deliver(gen, job.path, job.size, t);
                    }
                }
            });
        }
        Pool { q }
    }

    /// Replace whatever is still waiting with these jobs.
    pub fn request(&self, gen: u64, jobs: Vec<Job>) {
        let (m, cv) = &*self.q;
        let mut g = m.lock().unwrap();
        g.gen = gen;
        g.jobs = jobs.into();
        cv.notify_all();
    }
}

// ───────────── Windows: the shell ─────────────

#[cfg(windows)]
fn os_thumb(path: &Path, size: u32) -> Option<Thumb> {
    win::thumb(path, size)
}

#[cfg(windows)]
mod win {
    use super::Thumb;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::SIZE;

    /// The shell is still making the picture; not exported by the windows crate.
    const E_PENDING: windows::core::HRESULT = windows::core::HRESULT(0x8000_000Au32 as i32);
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
    };
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
    use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_THUMBNAILONLY};

    pub fn init_com() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        }
    }

    pub fn thumb(path: &Path, size: u32) -> Option<Thumb> {
        let ext = crate::fsx::ext_of(&crate::fsx::file_name(path));
        // apps and shortcuts: their icon is the picture; everything else: real thumbnails only
        let icon_ok = matches!(ext.as_str(), "exe" | "msi" | "lnk" | "url" | "appx" | "msix" | "cpl" | "scr" | "ttf" | "otf");
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            let factory: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).ok()?;
            let flags = if icon_ok { SIIGBF_BIGGERSIZEOK } else { SIIGBF_BIGGERSIZEOK | SIIGBF_THUMBNAILONLY };
            // The shell answers E_PENDING while it is still making the picture in the background.
            // Treating that as "no thumbnail" is why icons only turned up after F5; we are already
            // on a worker thread, so wait for it instead (~1.5 s in total, then give up).
            let sz = SIZE { cx: size as i32, cy: size as i32 };
            let mut img = None;
            for wait_ms in [20u64, 40, 80, 150, 300, 400, 500] {
                match factory.GetImage(sz, flags) {
                    Ok(b) => {
                        img = Some(b);
                        break;
                    }
                    Err(e) if e.code() == E_PENDING => std::thread::sleep(std::time::Duration::from_millis(wait_ms)),
                    Err(_) => return None,
                }
            }
            let hbmp = img?;
            let mut bm = BITMAP::default();
            let got = GetObjectW(HGDIOBJ(hbmp.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut BITMAP as *mut core::ffi::c_void));
            if got == 0 || bm.bmWidth <= 0 || bm.bmHeight == 0 {
                let _ = DeleteObject(HGDIOBJ(hbmp.0));
                return None;
            }
            let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
            let mut bi = BITMAPINFO::default();
            bi.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            };
            let mut buf = vec![0u8; (w * h * 4) as usize];
            let dc = CreateCompatibleDC(None);
            let lines = GetDIBits(dc, hbmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut core::ffi::c_void), &mut bi, DIB_RGB_COLORS);
            let _ = DeleteDC(dc);
            let _ = DeleteObject(HGDIOBJ(hbmp.0));
            if lines == 0 {
                return None;
            }
            // BGRA (premultiplied, or no alpha at all) → straight RGBA
            let no_alpha = buf.chunks_exact(4).all(|p| p[3] == 0);
            for px in buf.chunks_exact_mut(4) {
                px.swap(0, 2);
                if no_alpha {
                    px[3] = 255;
                } else if px[3] > 0 && px[3] < 255 {
                    let a = px[3] as u32;
                    for c in &mut px[..3] {
                        *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                    }
                }
            }
            // an all-black / fully transparent picture means the shell had nothing useful
            if buf.chunks_exact(4).all(|p| p[3] == 0 || (p[0] == 0 && p[1] == 0 && p[2] == 0)) {
                return None;
            }
            if let Ok(d) = std::env::var("SKJ_THUMB_DUMP") {
                if let Some(img) = image::RgbaImage::from_raw(w as u32, h as u32, buf.clone()) {
                    let _ = img.save(std::path::Path::new(&d).join(format!("{}.png", crate::fsx::file_name(path))));
                }
            }
            Some(Thumb { rgba: buf, w: w as u32, h: h as u32 })
        }
    }
}

// ───────────── Linux / macOS: caches + helper tools ─────────────

#[cfg(not(windows))]
fn load_png(p: &Path, size: u32) -> Option<Thumb> {
    let img = image::open(p).ok()?;
    let t = img.thumbnail(size, size).to_rgba8();
    let (w, h) = (t.width(), t.height());
    Some(Thumb { rgba: t.into_raw(), w, h })
}

#[cfg(not(windows))]
fn mtime(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).ok()?.modified().ok()
}

/// file:// URI the way GLib (and so the thumbnail spec) escapes it.
#[cfg(not(windows))]
pub fn file_uri(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut s = String::from("file://");
    for &b in p.as_os_str().as_bytes() {
        let ok = b.is_ascii_alphanumeric() || b"!$&'()*+,-./:=@_~".contains(&b);
        if ok {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{:02X}", b));
        }
    }
    s
}

#[cfg(not(windows))]
fn md5_hex(s: &str) -> String {
    use md5::{Digest, Md5};
    let d = Md5::digest(s.as_bytes());
    d.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(not(windows))]
fn our_cache() -> Option<PathBuf> {
    let d = dirs::cache_dir()?.join("skj-files").join("thumbs");
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

/// Run a helper tool, giving up after `secs`.
#[cfg(not(windows))]
fn run_tool(cmd: &mut std::process::Command, secs: u64) -> bool {
    use std::process::Stdio;
    let Ok(mut child) = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else { return false };
    let t0 = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return st.success(),
            Ok(None) if t0.elapsed().as_secs() < secs => std::thread::sleep(std::time::Duration::from_millis(30)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

#[cfg(not(windows))]
fn os_thumb(path: &Path, size: u32) -> Option<Thumb> {
    let file_time = mtime(path)?;
    let fresh = |t: &Path| mtime(t).map(|tt| tt >= file_time).unwrap_or(false);
    let uri = file_uri(path);
    let hash = md5_hex(&uri);
    // 1) the shared cache other apps filled
    #[cfg(not(target_os = "macos"))]
    if let Some(c) = dirs::cache_dir() {
        let order: &[&str] = if size > 256 { &["x-large", "xx-large", "large"] } else if size > 128 { &["large", "x-large", "normal"] } else { &["normal", "large"] };
        for d in order {
            let p = c.join("thumbnails").join(d).join(format!("{}.png", hash));
            if p.is_file() && fresh(&p) {
                if let Some(t) = load_png(&p, size) {
                    return Some(t);
                }
            }
        }
    }
    // 2) ours
    let cache = our_cache()?;
    let out = cache.join(format!("{}-{}.png", hash, size));
    if out.is_file() && fresh(&out) {
        return load_png(&out, size);
    }
    // 3) make one
    let ext = crate::fsx::ext_of(&crate::fsx::file_name(path));
    let kind = crate::fsx::kind_for_ext(&ext);
    let made = make_with_tools(path, &out, kind, &ext, size);
    if made && out.is_file() {
        return load_png(&out, size);
    }
    None
}

#[cfg(target_os = "macos")]
fn make_with_tools(path: &Path, out: &Path, _kind: i32, _ext: &str, size: u32) -> bool {
    use std::process::Command;
    let Some(dir) = out.parent() else { return false };
    let tmp = dir.join(format!("ql-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    let ok = run_tool(Command::new("qlmanage").arg("-t").arg("-s").arg(size.to_string()).arg("-o").arg(&tmp).arg(path), 10);
    let produced = tmp.join(format!("{}.png", crate::fsx::file_name(path)));
    let moved = ok && std::fs::rename(&produced, out).is_ok();
    let _ = std::fs::remove_dir_all(&tmp);
    moved
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn make_with_tools(path: &Path, out: &Path, kind: i32, ext: &str, size: u32) -> bool {
    use std::process::Command;
    if kind == crate::fsx::K_VIDEO {
        if run_tool(Command::new("ffmpegthumbnailer").arg("-i").arg(path).arg("-o").arg(out).arg("-s").arg(size.to_string()).arg("-c").arg("png"), 12) {
            return true;
        }
        let scale = format!("scale='min({s},iw)':'min({s},ih)':force_original_aspect_ratio=decrease", s = size);
        for seek in ["00:00:03", "00:00:00"] {
            if run_tool(
                Command::new("ffmpeg").args(["-y", "-loglevel", "error", "-ss", seek, "-i"]).arg(path).args(["-frames:v", "1", "-vf", &scale]).arg(out),
                12,
            ) && out.is_file()
            {
                return true;
            }
        }
        return false;
    }
    if ext == "pdf" {
        let stem = out.with_extension("");
        return run_tool(
            Command::new("pdftoppm").args(["-png", "-singlefile", "-f", "1", "-l", "1", "-scale-to"]).arg(size.to_string()).arg(path).arg(&stem),
            12,
        );
    }
    if matches!(ext, "svg" | "tif" | "tiff" | "heic" | "heif" | "avif" | "psd") {
        // ImageMagick handles most of these when it's installed
        let arg = format!("{}[0]", path.display());
        return run_tool(Command::new("magick").arg(&arg).arg("-thumbnail").arg(format!("{0}x{0}", size)).arg(out), 12)
            || run_tool(Command::new("convert").arg(&arg).arg("-thumbnail").arg(format!("{0}x{0}", size)).arg(out), 12);
    }
    false
}
