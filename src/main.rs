// SKJ Files — fast, good-looking file manager for Windows, Linux and macOS.
// No console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod i18n;
mod i18n_pl;
mod app;
mod fsx;
mod settings;
mod sysint;
mod thumbs;
mod update;
#[cfg(windows)]
mod winshell;

slint::include_modules!();

use std::time::Instant;

fn main() -> Result<(), slint::PlatformError> {
    let t0 = Instant::now();
    let timing = std::env::var("SKJ_TIMING").is_ok();

    // lets the installer notice we are open rather than failing to overwrite a locked .exe
    sysint::claim_running_marker();

    let s = settings::Settings::load();
    let ui = AppWindow::new()?;
    // language: before anything builds strings (Slint picks one by itself; override it with ours)
    app::apply_language(i18n::resolve(&s.lang));
    if timing { eprintln!("[skj-files] window created {:?}", t0.elapsed()); }
    app::wire(&ui);
    app::APP.with(|cell| *cell.borrow_mut() = Some(app::App::new(ui.clone_strong(), s)));
    let initial: Vec<std::path::PathBuf> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().to_string())
        .filter(|a| !a.starts_with("--") && !a.starts_with("-psn_"))
        .map(|a| {
            // Explorer passes drives as "C:\" — the quote rules turn that into `C:"`
            let a = a.trim_end_matches('"').to_string();
            if a.len() == 2 && a.ends_with(':') { format!("{}\\", a) } else { a }
        })
        .map(std::path::PathBuf::from)
        .collect();
    app::with_app(move |a| a.start(initial));

    if timing {
        eprintln!("[skj-files] ui ready in {:?}", t0.elapsed());
        let mut first = true;
        let _ = ui.window().set_rendering_notifier(move |state, _| {
            if first && matches!(state, slint::RenderingState::AfterRendering) {
                first = false;
                eprintln!("[skj-files] first frame after {:?}", t0.elapsed());
            }
        });
    }

    ui.run()?;

    app::with_app(|a| a.save_session());
    Ok(())
}
