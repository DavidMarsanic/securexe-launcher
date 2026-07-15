use crate::error::LauncherError;

/// Maps this machine's OS/arch to the orchestrator's `os-arch` target key
/// convention (see `securexe/lib/platform.ts` `TARGET_LABELS` on the website
/// repo — this must stay in sync with that naming).
pub fn target_key() -> Result<String, LauncherError> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        "windows" => "windows",
        other => {
            return Err(LauncherError::InvalidUrl(format!(
                "unsupported OS '{other}'"
            )))
        }
    };

    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => {
            return Err(LauncherError::InvalidUrl(format!(
                "unsupported arch '{other}'"
            )))
        }
    };

    Ok(format!("{os}-{arch}"))
}
