fn main() {
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .embed_resources(slint_build::EmbedResourcesKind::EmbedFiles)
        // Polish (and future languages): lang/<code>/LC_MESSAGES/skj-files.po, compiled into the binary.
        // No context: the English text alone is the key, so one string is translated once.
        .with_bundled_translations("lang")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    println!("cargo:rerun-if-changed=lang");
    slint_build::compile_with_config("ui/app.slint", config).expect("Slint build failed");

    // Windows: embed the app icon + version info into the .exe
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_windows_resources();
    }
}

#[cfg(windows)]
fn embed_windows_resources() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/skj-files.ico");
    res.set("ProductName", "SKJ Files");
    res.set("FileDescription", "SKJ Files");
    res.set("CompanyName", "SKJ Tech");
    let _ = res.compile();
    println!("cargo:rerun-if-changed=assets/skj-files.ico");
}

#[cfg(not(windows))]
fn embed_windows_resources() {
    // Cross-compiling from Linux: winresource is a target-only build dep, so do it by hand with windres.
    let out = std::env::var("OUT_DIR").unwrap();
    let ico = std::path::Path::new("assets/skj-files.ico");
    if !ico.exists() {
        return;
    }
    let rc = format!("{}/app.rc", out);
    let ico_abs = std::fs::canonicalize(ico).unwrap();
    std::fs::write(
        &rc,
        format!(
            "1 ICON \"{}\"\n1 VERSIONINFO\nFILEVERSION 0,1,0,0\nPRODUCTVERSION 0,1,0,0\nBEGIN\n BLOCK \"StringFileInfo\"\n BEGIN\n  BLOCK \"040904b0\"\n  BEGIN\n   VALUE \"ProductName\", \"SKJ Files\"\n   VALUE \"FileDescription\", \"SKJ Files\"\n   VALUE \"CompanyName\", \"SKJ Tech\"\n   VALUE \"FileVersion\", \"{ver}\"\n   VALUE \"ProductVersion\", \"{ver}\"\n  END\n END\n BLOCK \"VarFileInfo\"\n BEGIN\n  VALUE \"Translation\", 0x409, 1200\n END\nEND\n",
            ico_abs.display().to_string().replace('\\', "/"),
            ver = std::env::var("CARGO_PKG_VERSION").unwrap_or_default()
        ),
    )
    .unwrap();
    let obj = format!("{}/app_res.o", out);
    let ok = std::process::Command::new("x86_64-w64-mingw32-windres")
        .args([&rc, "-O", "coff", "-o", &obj])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        println!("cargo:rustc-link-arg-bins={}", obj);
    }
    println!("cargo:rerun-if-changed=assets/skj-files.ico");
}
