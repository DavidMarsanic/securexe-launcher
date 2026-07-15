use std::path::Path;
use std::process::Command;

use crate::error::LauncherError;

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), LauncherError> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(perms.mode() | 0o111);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), LauncherError> {
    Ok(())
}

/// Launches the downloaded, checksum-verified binary as a direct child
/// process — never through a shell, so nothing in the repo/commit/filename
/// can be interpreted as shell syntax.
pub fn launch(path: &Path) -> Result<(), LauncherError> {
    make_executable(path)?;
    Command::new(path).spawn()?;
    Ok(())
}
