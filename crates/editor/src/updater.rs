//! Updates from GitHub Releases.
//!
//! The overlay never touches the network. When `auto_update` is on, it starts
//! `crosslay-editor --update` once at startup, which runs this check in its
//! own short-lived process and exits. The editor window can also check and
//! install on request.
//!
//! A release carries the two exes as plain assets, named
//! `crosslay-windows-<arch>.exe` and `crosslay-editor-windows-<arch>.exe`.
//! Installing downloads both next to the running exes, stops the overlay,
//! swaps the files (a running exe can be renamed on Windows, just not
//! overwritten), and starts the overlay again.

// The GitHub parts only run in the Windows build; elsewhere they are
// compiled (and tested) but unused.
#![cfg_attr(not(windows), allow(dead_code))]

use std::path::{Path, PathBuf};

pub const REPO: &str = "tacobellerontop-sudo/crosslay";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// A release newer than the running version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub version: String,
    pub url: String,
    overlay: String,
    editor: String,
}

/// `"v1.2.3"` or `"1.2.3"` as comparable numbers. Pre-release suffixes
/// (`-beta`) are ignored, so they never count as newer.
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let v = (
        parts.next()??,
        parts.next().unwrap_or(Some(0))?,
        parts.next().unwrap_or(Some(0))?,
    );
    parts.next().is_none().then_some(v)
}

fn arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    }
}

pub fn asset_names() -> (String, String) {
    (
        format!("crosslay-windows-{}.exe", arch()),
        format!("crosslay-editor-windows-{}.exe", arch()),
    )
}

/// Pick the update out of a GitHub "latest release" response. `None` when it
/// isn't newer than `current` or lacks the assets for this machine.
pub fn release_from_json(json: &serde_json::Value, current: &str) -> Option<Release> {
    let tag = json["tag_name"].as_str()?;
    if json["draft"].as_bool() == Some(true) || json["prerelease"].as_bool() == Some(true) {
        return None;
    }
    if parse_version(tag)? <= parse_version(current)? {
        return None;
    }
    let (overlay_name, editor_name) = asset_names();
    let find = |name: &str| {
        json["assets"]
            .as_array()?
            .iter()
            .find(|a| a["name"].as_str() == Some(name))?["browser_download_url"]
            .as_str()
            .map(str::to_string)
    };
    Some(Release {
        tag: tag.to_string(),
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        url: json["html_url"].as_str().unwrap_or_default().to_string(),
        overlay: find(&overlay_name)?,
        editor: find(&editor_name)?,
    })
}

/// The folder the exes live in (and get replaced in).
pub fn install_dir() -> Result<PathBuf, String> {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
        .ok_or_else(|| "can't find the editor's folder".to_string())
}

/// Remove `*.old` exes left by a previous update. They can't be deleted while
/// they're still running, so this is retried on every start.
pub fn clean_up() {
    if let Ok(dir) = install_dir() {
        for name in ["crosslay.exe.old", "crosslay-editor.exe.old"] {
            let _ = std::fs::remove_file(dir.join(name));
        }
    }
}

/// Records the last release installed, so a release whose exes report the
/// wrong version can't make every overlay start reinstall it.
const MARKER: &str = "last-update.txt";

#[cfg(windows)]
mod net {
    use super::*;
    use std::io::Read;

    const MAX_DOWNLOAD: u64 = 64 << 20;

    fn agent() -> ureq::Agent {
        use ureq::tls::{TlsConfig, TlsProvider};
        ureq::Agent::config_builder()
            .tls_config(
                TlsConfig::builder()
                    .provider(TlsProvider::NativeTls)
                    .build(),
            )
            .timeout_global(Some(std::time::Duration::from_secs(60)))
            .user_agent(concat!("crosslay-updater/", env!("CARGO_PKG_VERSION")))
            .build()
            .into()
    }

    /// Ask GitHub for the latest release. `Ok(None)` = up to date.
    pub fn check() -> Result<Option<Release>, String> {
        let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
        let mut resp = match agent()
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .call()
        {
            Ok(r) => r,
            // No release published yet.
            Err(ureq::Error::StatusCode(404)) => return Ok(None),
            Err(e) => return Err(format!("couldn't reach GitHub: {e}")),
        };
        let mut body = String::new();
        resp.body_mut()
            .as_reader()
            .take(4 << 20)
            .read_to_string(&mut body)
            .map_err(|e| format!("couldn't read GitHub's answer: {e}"))?;
        let json: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| format!("unexpected answer from GitHub: {e}"))?;
        Ok(release_from_json(&json, CURRENT))
    }

    fn download(url: &str, to: &Path) -> Result<(), String> {
        let mut resp = agent()
            .get(url)
            .call()
            .map_err(|e| format!("download failed: {e}"))?;
        let bytes = resp
            .body_mut()
            .with_config()
            .limit(MAX_DOWNLOAD)
            .read_to_vec()
            .map_err(|e| format!("download failed: {e}"))?;
        if !bytes.starts_with(b"MZ") {
            return Err(format!("{url} isn't a Windows program"));
        }
        std::fs::write(to, bytes).map_err(|e| format!("can't write {}: {e}", to.display()))
    }

    /// Replace `target` with `new`, keeping the old file as `target.old`.
    fn swap(target: &Path, new: &Path) -> Result<(), String> {
        let mut old = target.as_os_str().to_owned();
        old.push(".old");
        let old = PathBuf::from(old);
        let _ = std::fs::remove_file(&old);
        let had_old = target.exists();
        if had_old {
            std::fs::rename(target, &old)
                .map_err(|e| format!("can't replace {}: {e}", target.display()))?;
        }
        std::fs::rename(new, target).map_err(|e| {
            if had_old {
                let _ = std::fs::rename(&old, target);
            }
            format!("can't replace {}: {e}", target.display())
        })
    }

    /// Download and install `rel`, restarting the overlay if it was running.
    /// `data_dir` is passed to the restarted overlay.
    pub fn install(rel: &Release, data_dir: &Path) -> Result<(), String> {
        let dir = install_dir()?;
        let overlay = dir.join("crosslay.exe");
        let editor = std::env::current_exe().map_err(|e| e.to_string())?;
        let overlay_new = dir.join("crosslay.exe.new");
        let editor_new = dir.join("crosslay-editor.exe.new");
        let result = download(&rel.overlay, &overlay_new)
            .and_then(|()| download(&rel.editor, &editor_new))
            .and_then(|()| {
                let was_running = crate::win::overlay_running();
                if was_running && !crate::win::stop_overlay(std::time::Duration::from_secs(5)) {
                    return Err("the overlay didn't stop, so it couldn't be replaced".into());
                }
                let swapped =
                    swap(&overlay, &overlay_new).and_then(|()| swap(&editor, &editor_new));
                if was_running {
                    let _ = std::process::Command::new(&overlay)
                        .arg("--dir")
                        .arg(data_dir)
                        .spawn();
                }
                swapped
            });
        let _ = std::fs::remove_file(&overlay_new);
        let _ = std::fs::remove_file(&editor_new);
        if result.is_ok() {
            let _ = std::fs::write(dir.join(MARKER), &rel.tag);
        }
        result
    }
}

#[cfg(windows)]
pub use net::{check, install};

#[cfg(not(windows))]
pub fn check() -> Result<Option<Release>, String> {
    Err("automatic updates are only available in the Windows build".into())
}

#[cfg(not(windows))]
pub fn install(_rel: &Release, _data_dir: &Path) -> Result<(), String> {
    Err("automatic updates are only available in the Windows build".into())
}

/// `crosslay-editor --update`: check, and install if there's something newer.
/// Quiet unless something goes wrong.
pub fn run_cli(data_dir: &Path) -> Result<(), String> {
    clean_up();
    let Some(rel) = check()? else {
        println!("crosslay {CURRENT} is up to date");
        return Ok(());
    };
    let installed = install_dir()
        .ok()
        .and_then(|d| std::fs::read_to_string(d.join(MARKER)).ok());
    if installed.as_deref().map(str::trim) == Some(rel.tag.as_str()) {
        println!("{} was already installed; skipping", rel.tag);
        return Ok(());
    }
    install(&rel, data_dir)?;
    println!("updated crosslay {CURRENT} -> {}", rel.version);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert_eq!(parse_version("v0.1.0"), Some((0, 1, 0)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("v1.2.3-beta.1"), Some((1, 2, 3)));
        assert_eq!(parse_version("nope"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert!(parse_version("v0.10.0") > parse_version("v0.9.9"));
    }

    fn release_json(tag: &str, prerelease: bool) -> serde_json::Value {
        let (o, e) = asset_names();
        serde_json::json!({
            "tag_name": tag,
            "draft": false,
            "prerelease": prerelease,
            "html_url": "https://example.invalid/r",
            "assets": [
                {"name": "notes.txt", "browser_download_url": "x"},
                {"name": o, "browser_download_url": "https://example.invalid/o.exe"},
                {"name": e, "browser_download_url": "https://example.invalid/e.exe"},
            ],
        })
    }

    #[test]
    fn picks_newer_releases_only() {
        let r = release_from_json(&release_json("v0.2.0", false), "0.1.0").unwrap();
        assert_eq!(r.version, "0.2.0");
        assert_eq!(r.overlay, "https://example.invalid/o.exe");
        assert_eq!(r.editor, "https://example.invalid/e.exe");
        assert!(release_from_json(&release_json("v0.1.0", false), "0.1.0").is_none());
        assert!(release_from_json(&release_json("v0.0.9", false), "0.1.0").is_none());
        assert!(release_from_json(&release_json("v0.2.0", true), "0.1.0").is_none());
        let mut no_assets = release_json("v0.2.0", false);
        no_assets["assets"] = serde_json::json!([]);
        assert!(release_from_json(&no_assets, "0.1.0").is_none());
    }
}
