//! System integration: making SKJ Files the default file manager (and undoing it).
//!
//! * Windows — per-user registry only (HKCU, no admin): folders and drives open in SKJ Files,
//!   and Win+E opens SKJ Files. Explorer itself stays installed and untouched.
//! * Linux — a .desktop file in ~/.local/share/applications + `xdg-mime default … inode/directory`.
//! * macOS — the Finder can't be replaced.

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegGetValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE,
        REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
    };

    const VERB: &str = "skjfiles";
    // "Home" (Windows 10/11 default for Win+E) and "This PC"
    const CLSIDS: [&str; 2] = ["{52205fd8-5dfb-447d-801a-d0b52f2e83e1}", "{20D04FE0-3AEA-1069-A2D8-08002B30309D}"];

    fn w(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    fn set(path: &str, name: Option<&str>, value: &str) -> Result<(), String> {
        unsafe {
            let mut key = HKEY::default();
            let p = w(path);
            RegCreateKeyExW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr()), None, None, REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None)
                .ok()
                .map_err(|e| format!("{}: {}", path, e.message()))?;
            let n = name.map(w);
            let v = w(value);
            let bytes = std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2);
            let r = RegSetValueExW(key, n.as_ref().map(|x| PCWSTR(x.as_ptr())).unwrap_or(PCWSTR::null()), None, REG_SZ, Some(bytes));
            let _ = RegCloseKey(key);
            r.ok().map_err(|e| format!("{}: {}", path, e.message()))
        }
    }

    fn get(path: &str, name: Option<&str>) -> Option<String> {
        unsafe {
            let p = w(path);
            let n = name.map(w);
            let mut buf = vec![0u16; 1024];
            let mut len = (buf.len() * 2) as u32;
            let r = RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(p.as_ptr()),
                n.as_ref().map(|x| PCWSTR(x.as_ptr())).unwrap_or(PCWSTR::null()),
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
                Some(&mut len),
            );
            if r.is_err() {
                return None;
            }
            let chars = (len as usize / 2).saturating_sub(1);
            Some(String::from_utf16_lossy(&buf[..chars.min(buf.len())]))
        }
    }

    fn del_tree(path: &str) {
        unsafe {
            let p = w(path);
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr()));
            let _ = windows::Win32::System::Registry::RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr()));
        }
    }

    fn del_value(path: &str, name: Option<&str>) {
        unsafe {
            let mut key = HKEY::default();
            let p = w(path);
            if windows::Win32::System::Registry::RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr()), None, KEY_READ | KEY_WRITE, &mut key).is_ok() {
                let n = name.map(w);
                let _ = RegDeleteValueW(key, n.as_ref().map(|x| PCWSTR(x.as_ptr())).unwrap_or(PCWSTR::null()));
                let _ = RegCloseKey(key);
            }
        }
    }

    pub fn supported() -> bool {
        true
    }

    pub fn is_default() -> bool {
        get(r"Software\Classes\Directory\shell", None).as_deref() == Some(VERB)
    }

    pub fn set_default(on: bool) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?.display().to_string();
        if on {
            for class in ["Directory", "Drive"] {
                let base = format!(r"Software\Classes\{}\shell", class);
                set(&format!(r"{}\{}", base, VERB), None, crate::i18n::ts("Open in SKJ Files"))?;
                set(&format!(r"{}\{}", base, VERB), Some("Icon"), &exe)?;
                set(&format!(r"{}\{}\command", base, VERB), None, &format!("\"{}\" \"%1\"", exe))?;
                set(&base, None, VERB)?;
            }
            for clsid in CLSIDS {
                let k = format!(r"Software\Classes\CLSID\{}\shell\opennewwindow\command", clsid);
                set(&k, None, &format!("\"{}\"", exe))?;
                set(&k, Some("DelegateExecute"), "")?;
            }
        } else {
            for class in ["Directory", "Drive"] {
                let base = format!(r"Software\Classes\{}\shell", class);
                del_tree(&format!(r"{}\{}", base, VERB));
                if get(&base, None).as_deref() == Some(VERB) {
                    del_value(&base, None);
                }
            }
            for clsid in CLSIDS {
                del_tree(&format!(r"Software\Classes\CLSID\{}\shell\opennewwindow", clsid));
            }
        }
        Ok(())
    }

    pub fn previous_default() -> String {
        String::new()
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use std::process::Command;

    fn desktop_file() -> Option<std::path::PathBuf> {
        Some(dirs::data_dir()?.join("applications").join("skj-files.desktop"))
    }

    pub fn supported() -> bool {
        Command::new("xdg-mime").arg("--version").output().is_ok()
    }

    fn current() -> String {
        Command::new("xdg-mime")
            .args(["query", "default", "inode/directory"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    }

    pub fn previous_default() -> String {
        let c = current();
        if c == "skj-files.desktop" { String::new() } else { c }
    }

    pub fn is_default() -> bool {
        current() == "skj-files.desktop"
    }

    /// Writes (or refreshes) ~/.local/share/applications/skj-files.desktop.
    pub fn install_desktop_entry() -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let f = desktop_file().ok_or("no data folder")?;
        if let Some(d) = f.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        // icon next to the desktop file
        let icon_dir = dirs::data_dir().ok_or("no data folder")?.join("icons/hicolor/256x256/apps");
        let _ = std::fs::create_dir_all(&icon_dir);
        let _ = std::fs::write(icon_dir.join("skj-files.png"), include_bytes!("../assets/skj-files.png"));
        let text = format!(
            "[Desktop Entry]\nType=Application\nName=SKJ Files\nGenericName=File Manager\nComment=Fast file manager\nExec=\"{}\" %F\nIcon=skj-files\nTerminal=false\nCategories=System;FileManager;Utility;\nMimeType=inode/directory;\nStartupWMClass=skj-files\n",
            exe.display()
        );
        std::fs::write(&f, text).map_err(|e| e.to_string())?;
        let _ = Command::new("update-desktop-database").arg(f.parent().unwrap()).output();
        Ok(())
    }

    pub fn set_default(on: bool, prev: &str) -> Result<(), String> {
        if on {
            install_desktop_entry()?;
            let st = Command::new("xdg-mime").args(["default", "skj-files.desktop", "inode/directory"]).status().map_err(|e| e.to_string())?;
            if !st.success() {
                return Err("xdg-mime failed".into());
            }
        } else {
            let back = if prev.is_empty() { "org.kde.dolphin.desktop" } else { prev };
            let _ = Command::new("xdg-mime").args(["default", back, "inode/directory"]).status();
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    pub fn supported() -> bool {
        false
    }
    pub fn is_default() -> bool {
        false
    }
    pub fn previous_default() -> String {
        String::new()
    }
}

pub use imp::{is_default, previous_default, supported};

/// Make SKJ Files the default file manager (`on`) or give the job back.
pub fn set_default(on: bool, prev: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = prev;
        imp::set_default(on)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        imp::set_default(on, prev)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = (on, prev);
        Err(crate::i18n::ts("macOS doesn't let another app replace the Finder").into())
    }
}
