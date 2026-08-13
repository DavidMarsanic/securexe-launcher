use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::LauncherError;

/// A previously-downloaded, checksum-verified app. `path` and `icon` are
/// local filesystem paths — never sent to the frontend as-is, since the
/// webview has no business seeing raw disk layout (see `gallery` in lib.rs
/// for the sanitized shape that actually crosses that boundary).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryEntry {
    pub slug: String,
    pub repo: String,
    pub commit: String,
    pub path: PathBuf,
    pub is_gui: bool,
    pub icon: Option<PathBuf>,
    pub installed_at: u64,
    pub last_launched_at: Option<u64>,
}

fn library_path() -> Result<PathBuf, LauncherError> {
    let home = dirs::home_dir()
        .ok_or_else(|| LauncherError::Io("could not resolve home directory".into()))?;
    Ok(home.join(".securexe").join("library.json"))
}

pub fn load() -> Result<Vec<LibraryEntry>, LauncherError> {
    let path = library_path()?;
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let data = std::fs::read_to_string(&path)?;
    serde_json::from_str(&data).map_err(|e| LauncherError::Io(format!("bad library.json: {e}")))
}

fn save(entries: &[LibraryEntry]) -> Result<(), LauncherError> {
    let path = library_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(entries)
        .map_err(|e| LauncherError::Io(format!("failed to serialize library: {e}")))?;
    std::fs::write(&path, data)?;
    Ok(())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Records (or updates) an app after a successful install. Called on every
/// run — including cache hits — so `path`/`is_gui`/`icon` stay correct even
/// if a repo's build shape changes between visits (e.g. fberadicator's
/// darwin target switching from a bare binary to a `.app.zip`).
pub fn record_install(
    slug: &str,
    repo: &str,
    commit: &str,
    path: PathBuf,
    is_gui: bool,
    icon: Option<PathBuf>,
) -> Result<(), LauncherError> {
    let mut entries = load()?;
    if let Some(existing) = entries.iter_mut().find(|e| e.slug == slug) {
        existing.repo = repo.to_string();
        existing.commit = commit.to_string();
        existing.path = path;
        existing.is_gui = is_gui;
        existing.icon = icon;
    } else {
        entries.push(LibraryEntry {
            slug: slug.to_string(),
            repo: repo.to_string(),
            commit: commit.to_string(),
            path,
            is_gui,
            icon,
            installed_at: now(),
            last_launched_at: None,
        });
    }
    save(&entries)
}

/// Looks up a previously-installed entry by slug — used by the gallery's
/// "relaunch" action, which re-runs the cached, already-verified binary
/// directly rather than requiring a fresh signed link for every replay.
pub fn find(slug: &str) -> Result<Option<LibraryEntry>, LauncherError> {
    Ok(load()?.into_iter().find(|e| e.slug == slug))
}

pub fn touch_last_launched(slug: &str) -> Result<(), LauncherError> {
    let mut entries = load()?;
    if let Some(existing) = entries.iter_mut().find(|e| e.slug == slug) {
        existing.last_launched_at = Some(now());
        save(&entries)?;
    }
    Ok(())
}

/// Drops `slug` from the library. Does not touch anything on disk — see
/// `install::remove_all` for deleting the actual downloaded files.
pub fn remove(slug: &str) -> Result<(), LauncherError> {
    let entries = load()?;
    let filtered: Vec<_> = entries.into_iter().filter(|e| e.slug != slug).collect();
    save(&filtered)
}
