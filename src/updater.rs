//! Auto-update for players you send the game to.
//!
//! A released copy of the game has an `update.cfg` next to the executable
//! holding the address the releases are published at (written by
//! `tools/release.ps1`). On start, a background thread fetches
//! `manifest.txt` from there:
//!
//! ```text
//! version = 0.2.0
//! assets = <hash of the assets folder>
//! ```
//!
//! If the version is newer it downloads the new `walkabout.exe` (and
//! `assets.zip`, only when the assets changed), swaps the executable (a
//! running .exe can be renamed on Windows) and offers a restart. Builds
//! without `update.cfg` (e.g. `cargo run`) never update.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Checking,
    UpToDate,
    Downloading { what: &'static str, done: u64, total: u64 },
    /// Installed; takes effect after a restart.
    Ready(String),
    Failed(String),
}

impl Status {
    /// A line for the menu.
    pub fn describe(&self) -> String {
        match self {
            Status::Checking => "Checking for updates...".into(),
            Status::UpToDate => format!("Version {CURRENT}: up to date"),
            Status::Downloading { what, done, total } => {
                let pct = if *total > 0 { *done as f32 / *total as f32 * 100.0 } else { 0.0 };
                format!("Downloading the update ({what}): {pct:.0}%")
            }
            Status::Ready(v) => format!("Version {v} installed: restart to play it"),
            Status::Failed(e) => format!("Update check failed: {e}"),
        }
    }
}

pub struct Updater {
    status: Arc<Mutex<Status>>,
}

impl Updater {
    /// Starts checking in the background, if this copy is set up to update.
    pub fn start() -> Option<Self> {
        let dir = exe_dir()?;
        cleanup(&dir);
        let base = read_url(&dir.join("update.cfg"))?;
        let status = Arc::new(Mutex::new(Status::Checking));
        let shared = status.clone();
        std::thread::spawn(move || {
            let result = run(&base, &dir, &shared);
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            match result {
                Ok(Some(v)) => *s = Status::Ready(v),
                Ok(None) => *s = Status::UpToDate,
                Err(e) => {
                    log::warn!("update: {e:#}");
                    *s = Status::Failed(format!("{e}"));
                }
            }
        });
        Some(Self { status })
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Starts the (now updated) game again and ends this one.
pub fn restart() -> ! {
    if let Ok(exe) = std::env::current_exe() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let _ = std::process::Command::new(exe).args(args).spawn();
    }
    std::process::exit(0);
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

/// Removes the previous executable left over from the last update.
fn cleanup(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.path().extension().is_some_and(|x| x == "old") {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

/// The `url = ...` line of update.cfg, ending in a slash.
fn read_url(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let url = text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == "url").then(|| v.trim().to_owned())
    })?;
    if url.is_empty() {
        return None;
    }
    Some(if url.ends_with('/') { url } else { format!("{url}/") })
}

/// `key = value` from the manifest.
fn manifest_value<'a>(manifest: &'a str, key: &str) -> Option<&'a str> {
    manifest.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == key).then(|| v.trim())
    })
}

/// True if version `a` is newer than `b` ("1.2.10" > "1.2.9").
pub fn newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> { v.trim().trim_start_matches('v').split('.').map(|n| n.parse().unwrap_or(0)).collect() };
    parse(a) > parse(b)
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .user_agent("walkabout-updater")
        .build()
        .into()
}

/// Returns the version installed, or None when already up to date.
fn run(base: &str, dir: &Path, status: &Mutex<Status>) -> Result<Option<String>> {
    let manifest = agent(Duration::from_secs(15))
        .get(&format!("{base}manifest.txt"))
        .call()
        .context("couldn't reach the update server")?
        .body_mut()
        .read_to_string()?;
    let Some(version) = manifest_value(&manifest, "version") else {
        bail!("the update manifest has no version");
    };
    if !newer(version, CURRENT) {
        return Ok(None);
    }
    log::info!("update: {CURRENT} -> {version}");

    let exe = std::env::current_exe()?;
    let new_exe = dir.join("walkabout.exe.download");
    download(&format!("{base}walkabout.exe"), &new_exe, "game", status)?;

    // Assets only when they changed (they're the big part).
    let assets = dir.join("assets");
    let local_hash = fs::read_to_string(assets.join(".hash")).unwrap_or_default();
    if let Some(hash) = manifest_value(&manifest, "assets")
        && hash != local_hash.trim()
    {
        let zip_path = dir.join("assets.zip.download");
        download(&format!("{base}assets.zip"), &zip_path, "assets", status)?;
        let file = fs::File::open(&zip_path)?;
        zip::ZipArchive::new(file)
            .context("the downloaded assets are damaged")?
            .extract(dir)
            .context("couldn't unpack the assets")?;
        let _ = fs::remove_file(&zip_path);
        let unpacked = fs::read_to_string(assets.join(".hash")).unwrap_or_default();
        if unpacked.trim() != hash {
            bail!("the assets download was incomplete");
        }
    }

    // Swap the executable: the running one can be renamed, not replaced.
    let old = exe.with_extension("exe.old");
    let _ = fs::remove_file(&old);
    fs::rename(&exe, &old).context("couldn't move the old game aside")?;
    if let Err(e) = fs::rename(&new_exe, &exe) {
        let _ = fs::rename(&old, &exe); // put it back
        return Err(e).context("couldn't install the new game");
    }
    Ok(Some(version.to_owned()))
}

/// Streams `url` to `path`, reporting progress.
fn download(url: &str, path: &Path, what: &'static str, status: &Mutex<Status>) -> Result<()> {
    let response = agent(Duration::from_secs(30 * 60))
        .get(url)
        .call()
        .with_context(|| format!("couldn't download the {what}"))?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut reader = response.into_body().into_reader();
    let mut file = fs::File::create(path)?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        *status.lock().unwrap_or_else(|e| e.into_inner()) = Status::Downloading { what, done, total };
    }
    file.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_numerically() {
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("1.10.0", "1.9.3"));
        assert!(newer("v0.1.1", "0.1.0"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.0.9", "0.1.0"));
    }

    #[test]
    fn reads_the_manifest_and_config() {
        let m = "version = 0.3.0\nassets = abc123\n";
        assert_eq!(manifest_value(m, "version"), Some("0.3.0"));
        assert_eq!(manifest_value(m, "assets"), Some("abc123"));
        let dir = std::env::temp_dir().join("walkabout-updater-test");
        let _ = fs::create_dir_all(&dir);
        let cfg = dir.join("update.cfg");
        fs::write(&cfg, "# where updates come from\nurl = https://example.com/releases/latest/download\n").unwrap();
        assert_eq!(read_url(&cfg).as_deref(), Some("https://example.com/releases/latest/download/"));
        let _ = fs::remove_dir_all(&dir);
    }
}
