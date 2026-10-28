//! Crash log. A release build on Windows has no console (`windows_subsystem = "windows"`), so a
//! panic would otherwise take the window away with nothing to show for it. This writes the panic,
//! the version and the place it happened to a file next to the settings, keeps the last few, and
//! lets Settings → System open the folder.

use std::io::Write;
use std::path::PathBuf;

/// Where the logs live: the same folder as settings.ini.
pub fn dir() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("skj-files").join("crashes"))
}

/// Newest first.
pub fn list() -> Vec<PathBuf> {
    let Some(d) = dir() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(&d) else { return Vec::new() };
    let mut v: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "txt"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().map(|(_, p)| p).collect()
}

/// Install the panic hook. Call once, as early in main as possible.
pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write(info);
        previous(info);
    }));
}

fn write(info: &std::panic::PanicHookInfo<'_>) {
    let Some(d) = dir() else { return };
    if std::fs::create_dir_all(&d).is_err() {
        return;
    }

    // A panic is already a bad day; never panic again while reporting one.
    let when = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let path = d.join(format!("crash-{}.txt", when));
    let Ok(mut f) = std::fs::File::create(&path) else { return };

    let msg = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".into());
    let at = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "(unknown place)".into());

    let _ = writeln!(f, "SKJ Files {} crashed", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(f, "when:  {}", when);
    let _ = writeln!(f, "os:    {} {}", std::env::consts::OS, std::env::consts::ARCH);
    let _ = writeln!(f, "where: {}", at);
    let _ = writeln!(f, "what:  {}", msg);
    let _ = writeln!(f, "\n--- backtrace ---");
    let _ = writeln!(f, "{}", std::backtrace::Backtrace::force_capture());

    prune(&d);
}

/// Keep the newest 10, so this can never grow without bound.
fn prune(d: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(d) else { return };
    let mut v: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "txt"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if v.len() <= 10 {
        return;
    }
    v.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, p) in v.into_iter().skip(10) {
        let _ = std::fs::remove_file(p);
    }
}
