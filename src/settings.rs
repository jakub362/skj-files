//! Tiny key=value settings file, no extra dependencies.

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Settings {
    pub dark: bool,
    pub accent: usize,
    pub show_hidden: bool,
    pub show_ext: bool,
    pub restore: bool,
    pub confirm_delete: bool,
    pub view_mode: i32,
    pub sort_key: i32,
    pub sort_asc: bool,
    pub show_preview: bool,
    pub pins: Vec<PathBuf>,
    pub pins_set: bool,
    pub recent: Vec<PathBuf>,
    pub tabs: Vec<String>,
    pub active_tab: usize,
    pub friendly_dates: bool,
    pub grid_size: i32,
    pub tags: Vec<(PathBuf, u8)>,
    /// Details view column widths: date modified, date created, type, size, location
    pub col_w: [f32; 5],
    /// Which optional columns are shown: date modified, date created, type, size
    pub col_show: [bool; 4],
    /// Saved network places (\\server\share, smb://…)
    pub servers: Vec<String>,
    /// Files opened from SKJ Files, newest first
    pub recent_files: Vec<PathBuf>,
    /// Toolbar buttons that are shown (ids, see app::TOOLBAR)
    pub toolbar: Vec<String>,
    pub toolbar_set: bool,
    /// Custom keyboard shortcuts: (action, combo like "Ctrl+Shift+K")
    pub shortcuts: Vec<(String, String)>,
    /// "auto", "en" or "pl"
    pub lang: String,
    /// The file manager that was the default before SKJ Files took over (Linux)
    pub prev_fm: String,
}

pub const DEFAULT_COL_W: [f32; 5] = [140.0, 140.0, 130.0, 76.0, 220.0];

impl Default for Settings {
    fn default() -> Self {
        Settings {
            dark: true,
            accent: 0,
            show_hidden: false,
            show_ext: true,
            restore: true,
            confirm_delete: true,
            view_mode: 0,
            sort_key: 0,
            sort_asc: true,
            show_preview: true,
            pins: Vec::new(),
            pins_set: false,
            recent: Vec::new(),
            tabs: Vec::new(),
            active_tab: 0,
            friendly_dates: true,
            grid_size: 1,
            tags: Vec::new(),
            col_w: DEFAULT_COL_W,
            col_show: [true, false, true, true],
            servers: Vec::new(),
            recent_files: Vec::new(),
            toolbar: Vec::new(),
            toolbar_set: false,
            shortcuts: Vec::new(),
            lang: "auto".into(),
            prev_fm: String::new(),
        }
    }
}

fn file() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("skj-files").join("settings.ini"))
}

impl Settings {
    pub fn load() -> Settings {
        let mut s = Settings::default();
        let Some(f) = file() else { return s };
        let Ok(txt) = std::fs::read_to_string(f) else { return s };
        for line in txt.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let b = v == "1";
            match k {
                "dark" => s.dark = b,
                "accent" => s.accent = v.parse().unwrap_or(0),
                "hidden" => s.show_hidden = b,
                "ext" => s.show_ext = b,
                "restore" => s.restore = b,
                "confirm" => s.confirm_delete = b,
                "view" => s.view_mode = v.parse().unwrap_or(0),
                "sort" => s.sort_key = v.parse().unwrap_or(0),
                "asc" => s.sort_asc = b,
                "preview" => s.show_preview = b,
                "pins_set" => s.pins_set = b,
                "pin" => s.pins.push(PathBuf::from(v)),
                "recent" => s.recent.push(PathBuf::from(v)),
                "tab" => s.tabs.push(v.to_string()),
                "active_tab" => s.active_tab = v.parse().unwrap_or(0),
                "friendly" => s.friendly_dates = b,
                "grid_size" => s.grid_size = v.parse().unwrap_or(1),
                "cols" => {
                    for (i, x) in v.split(',').enumerate().take(5) {
                        if let Ok(w) = x.parse::<f32>() {
                            s.col_w[i] = w.clamp(48.0, 700.0);
                        }
                    }
                }
                "colshow" => {
                    for (i, x) in v.split(',').enumerate().take(4) {
                        s.col_show[i] = x == "1";
                    }
                }
                "server" => s.servers.push(v.to_string()),
                "recentfile" => s.recent_files.push(PathBuf::from(v)),
                "toolbar" => {
                    s.toolbar_set = true;
                    s.toolbar = v.split(',').filter(|x| !x.is_empty()).map(|x| x.to_string()).collect();
                }
                "shortcut" => {
                    if let Some((a, c)) = v.split_once('|') {
                        s.shortcuts.push((a.to_string(), c.to_string()));
                    }
                }
                "lang" => s.lang = v.to_string(),
                "prev_fm" => s.prev_fm = v.to_string(),
                "tag" => {
                    if let Some((m, path)) = v.split_once('|') {
                        if let Ok(m) = m.parse::<u8>() {
                            s.tags.push((PathBuf::from(path), m));
                        }
                    }
                }
                _ => {}
            }
        }
        s
    }

    pub fn save(&self) {
        let Some(f) = file() else { return };
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let b = |x: bool| if x { "1" } else { "0" };
        let mut out = String::new();
        out.push_str(&format!("dark={}\naccent={}\nhidden={}\next={}\nrestore={}\nconfirm={}\nview={}\nsort={}\nasc={}\npreview={}\npins_set={}\nactive_tab={}\n",
            b(self.dark), self.accent, b(self.show_hidden), b(self.show_ext), b(self.restore), b(self.confirm_delete),
            self.view_mode, self.sort_key, b(self.sort_asc), b(self.show_preview), b(self.pins_set), self.active_tab));
        for p in &self.pins {
            out.push_str(&format!("pin={}\n", p.display()));
        }
        for p in &self.recent {
            out.push_str(&format!("recent={}\n", p.display()));
        }
        for t in &self.tabs {
            out.push_str(&format!("tab={}\n", t));
        }
        out.push_str(&format!("friendly={}\ngrid_size={}\n", b(self.friendly_dates), self.grid_size));
        out.push_str(&format!("cols={}\ncolshow={}\n",
            self.col_w.iter().map(|w| format!("{:.0}", w)).collect::<Vec<_>>().join(","),
            self.col_show.iter().map(|x| b(*x)).collect::<Vec<_>>().join(",")));
        for x in &self.servers {
            out.push_str(&format!("server={}\n", x));
        }
        for p in &self.recent_files {
            out.push_str(&format!("recentfile={}\n", p.display()));
        }
        if self.toolbar_set {
            out.push_str(&format!("toolbar={}\n", self.toolbar.join(",")));
        }
        for (a, c) in &self.shortcuts {
            out.push_str(&format!("shortcut={}|{}\n", a, c));
        }
        out.push_str(&format!("lang={}\nprev_fm={}\n", self.lang, self.prev_fm));
        for (p, m) in &self.tags {
            if *m != 0 {
                out.push_str(&format!("tag={}|{}\n", m, p.display()));
            }
        }
        // write next to it, then swap in, so a crash mid-write never leaves a broken file
        let tmp = f.with_extension("ini.tmp");
        if std::fs::write(&tmp, out).is_ok() {
            if f.exists() {
                let _ = std::fs::copy(&f, f.with_extension("ini.bak"));
            }
            let _ = std::fs::rename(&tmp, &f);
        }
    }
}
