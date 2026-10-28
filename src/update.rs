//! Update check: asks GitHub whether a newer release is out.
//!
//! It only ever *reads* — nothing is downloaded or installed behind the user's back. When a newer
//! release exists the app shows a toast and offers to open the release page in the browser, which
//! is also the only thing the "Check for updates" button does.
//!
//! The request is plain HTTPS to the public releases API, with no token and no user data attached.
//!
//! Planned for later (Jakub, 2026-10-28): download the matching installer in the background, then
//! tell the user it will be applied when they restart, rather than interrupting them. The pieces
//! that already exist for it: release assets are named `...-windows-x64-setup.exe` so the right
//! one can be picked by name, the installer accepts `/VERYSILENT`, and the app holds the
//! `SKJFilesSingleInstance` mutex so the installer can tell it is running. The missing piece is
//! scheduling the run — a `RunOnce` registry value pointing at the downloaded setup is the usual
//! way, since it fires at the next sign-in and the app is guaranteed not to be running then.

use std::sync::atomic::{AtomicBool, Ordering};

/// Where releases are published, as "owner/name", taken from `repository` in Cargo.toml so that
/// moving the project (to the SKJ-Technology org, say) is a one-line change there and nothing here.
pub fn repo() -> &'static str {
    env!("CARGO_PKG_REPOSITORY")
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .rsplit("github.com/")
        .next()
        .unwrap_or("")
}

pub fn releases_url() -> String {
    format!("https://github.com/{}/releases/latest", repo())
}

/// A newer release than the one running.
#[derive(Clone, Debug)]
pub struct Newer {
    /// The tag as GitHub has it, e.g. "v0.5.0"
    pub tag: String,
    /// Just the numbers, e.g. "0.5.0"
    pub version: String,
}

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Ask GitHub in the background. `done` runs on a worker thread with `Some(newer)` only when the
/// published release is actually newer than this build; `None` means up to date or unreachable.
///
/// A failure is deliberately silent: no network, a proxy, an offline laptop or GitHub being down
/// are all normal and none of them are worth a popup.
pub fn check_async(done: impl FnOnce(Option<Newer>) + Send + 'static) {
    // one at a time, so a held-down "Check now" cannot pile up threads
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let found = check_blocking();
        RUNNING.store(false, Ordering::SeqCst);
        done(found);
    });
}

fn check_blocking() -> Option<Newer> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo());
    let body = ureq::get(&url)
        .header("User-Agent", &format!("skj-files/{}", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    let tag = json_string(&body, "tag_name")?;
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    if is_newer(&version, env!("CARGO_PKG_VERSION")) {
        Some(Newer { tag, version })
    } else {
        None
    }
}

/// Pull one string value out of a flat JSON object. Enough for `"tag_name": "v0.5.0"` and avoids
/// pulling a JSON parser in for a single field. Handles the escapes GitHub can put in a tag.
fn json_string(body: &str, key: &str) -> Option<String> {
    let at = body.find(&format!("\"{}\"", key))?;
    let rest = &body[at + key.len() + 2..];
    let rest = rest.trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    let n = u32::from_str_radix(&hex, 16).ok()?;
                    out.push(char::from_u32(n).unwrap_or('?'));
                }
                other => out.push(other),
            },
            _ => out.push(c),
        }
    }
    None
}

/// Compare dotted numbers, e.g. "0.10.0" is newer than "0.9.3". Anything after the numbers
/// (`0.5.0-beta1`) is ignored, so a pre-release of the running version does not count as newer.
fn is_newer(candidate: &str, running: &str) -> bool {
    let nums = |s: &str| -> Vec<u64> {
        s.split(['.', '-', '+'])
            .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
            .take_while(|p| !p.is_empty())
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    let (a, b) = (nums(candidate), nums(running));
    if a.is_empty() {
        return false;
    }
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_versions() {
        assert!(is_newer("0.5.0", "0.4.0"));
        assert!(is_newer("0.10.0", "0.9.3"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(is_newer("0.4.1", "0.4.0"));
    }

    #[test]
    fn not_newer() {
        assert!(!is_newer("0.4.0", "0.4.0"));
        assert!(!is_newer("0.3.9", "0.4.0"));
        assert!(!is_newer("0.9.3", "0.10.0"));
        // a pre-release of what is already running is not an update
        assert!(!is_newer("0.4.0-beta1", "0.4.0"));
        assert!(!is_newer("", "0.4.0"));
        assert!(!is_newer("not a version", "0.4.0"));
    }

    #[test]
    fn repo_comes_from_cargo_toml() {
        // the whole point: moving the project is one line in Cargo.toml
        assert_eq!(repo(), "jakub362/skj-files");
        assert!(releases_url().ends_with("/releases/latest"));
    }

    #[test]
    fn reads_the_tag() {
        let body = r#"{"url":"x","tag_name":"v0.5.0","name":"SKJ Files 0.5.0"}"#;
        assert_eq!(json_string(body, "tag_name").as_deref(), Some("v0.5.0"));
        assert_eq!(json_string(body, "missing"), None);
    }

    #[test]
    fn reads_a_tag_with_escapes() {
        let body = r#"{"tag_name":"v0.5.0+1","x":1}"#;
        assert_eq!(json_string(body, "tag_name").as_deref(), Some("v0.5.0+1"));
    }

    #[test]
    fn spaces_around_the_colon() {
        let body = "{\"tag_name\" : \"v1.2.3\"}";
        assert_eq!(json_string(body, "tag_name").as_deref(), Some("v1.2.3"));
    }
}
