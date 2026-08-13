use std::path::PathBuf;

use crate::error::LauncherError;
use crate::verify;

fn securexe_home() -> Result<PathBuf, LauncherError> {
    let home = dirs::home_dir()
        .ok_or_else(|| LauncherError::Io("could not resolve home directory".into()))?;
    Ok(home.join(".securexe"))
}

/// `~/.securexe/apps/<slug>/` — every downloaded build of an app, across
/// all commits. `slug` is already validated in repo.rs (safe charset, no
/// `..`) before reaching here.
pub fn app_dir(slug: &str) -> Result<PathBuf, LauncherError> {
    Ok(securexe_home()?.join("apps").join(slug))
}

/// `~/.securexe/apps/<slug>/<commit>/<file>`.
pub fn artifact_path(slug: &str, commit: &str, file: &str) -> Result<PathBuf, LauncherError> {
    Ok(app_dir(slug)?.join(commit).join(file))
}

/// `~/.securexe/icons/<slug>.png` — where a bundle's converted icon is
/// cached for the gallery, keyed by slug rather than commit since an icon
/// is treated as belonging to the app, not a specific build of it.
pub fn icon_path(slug: &str) -> Result<PathBuf, LauncherError> {
    Ok(securexe_home()?.join("icons").join(format!("{slug}.png")))
}

/// `~/.securexe/sandbox/<slug>/` — the working directory a launched app
/// runs in. Never the user's real `$HOME` or wherever the launcher process
/// itself happened to start: a CLI tool that treats "current directory" as
/// its target (a code counter, a backup tool, anything that reads *or
/// writes* relative to cwd) would otherwise silently operate on the user's
/// actual home folder. Persistent per-slug (not a fresh temp dir per
/// launch) so a tool's own output survives between runs.
pub fn sandbox_dir(slug: &str) -> Result<PathBuf, LauncherError> {
    let dir = securexe_home()?.join("sandbox").join(slug);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Removes everything on disk belonging to `slug` — downloaded builds,
/// cached icon, sandbox contents. Best-effort per path: a missing
/// directory isn't an error, since uninstall should succeed even if the
/// library entry and the files it points at have already drifted apart.
pub fn remove_all(slug: &str) -> Result<(), LauncherError> {
    for path in [app_dir(slug)?, securexe_home()?.join("sandbox").join(slug)] {
        if path.is_dir() {
            std::fs::remove_dir_all(&path)?;
        }
    }
    let icon = icon_path(slug)?;
    if icon.is_file() {
        std::fs::remove_file(&icon)?;
    }
    Ok(())
}

/// True if `path` already exists and its sha256 matches `expected_sha256`.
/// A cache hit lets us skip the download entirely.
pub fn is_cached(path: &std::path::Path, expected_sha256: &str) -> bool {
    if !path.is_file() {
        return false;
    }
    match verify::sha256_file(path) {
        Ok(actual) => actual.eq_ignore_ascii_case(expected_sha256),
        Err(_) => false,
    }
}
