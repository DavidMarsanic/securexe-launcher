use std::path::PathBuf;

use crate::error::LauncherError;
use crate::verify;

/// `~/.securexe/apps/<slug>/<commit>/<file>` — `slug` and `commit` are
/// already validated in repo.rs (safe charset, no `..`) before reaching here.
pub fn artifact_path(slug: &str, commit: &str, file: &str) -> Result<PathBuf, LauncherError> {
    let home = dirs::home_dir()
        .ok_or_else(|| LauncherError::Io("could not resolve home directory".into()))?;
    Ok(home.join(".securexe").join("apps").join(slug).join(commit).join(file))
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
