//! Checks whether a newer release of securexe-launcher itself is
//! available — distinct from the per-app update badges in lib.rs, which
//! check installed *applets* against the orchestrator. This checks the
//! launcher's own GitHub Releases, which is where release.yml already
//! publishes real builds (draft-first, so this only ever sees a release
//! someone actually clicked "Publish" on — the GitHub API's
//! `/releases/latest` endpoint excludes drafts and prereleases by
//! design). Notification only: this tells the user a newer version
//! exists and links to it, it doesn't download or install anything
//! itself.

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::error::LauncherError;

const RELEASES_LATEST_URL: &str =
    "https://api.github.com/repos/DavidMarsanic/securexe-launcher/releases/latest";

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
}

/// What the frontend needs to show the notification and let the user act
/// on it.
#[derive(Serialize)]
pub struct LauncherUpdate {
    pub version: String,
    pub download_url: String,
}

/// Compares `current_version` (from `app.package_info().version`, itself
/// stamped from the release tag at build time — see release.yml) against
/// the latest published release tag. Returns `None` if already current,
/// on the latest, or ahead of it (a locally-built dev binary reporting
/// some version newer than anything released shouldn't nag either).
pub async fn check(current_version: &str) -> Result<Option<LauncherUpdate>, LauncherError> {
    let current = Version::parse(current_version)
        .map_err(|e| LauncherError::Io(format!("bad current version {current_version:?}: {e}")))?;

    let client = reqwest::Client::builder()
        .user_agent("securexe-launcher")
        .build()?;
    let resp = client.get(RELEASES_LATEST_URL).send().await?;
    if !resp.status().is_success() {
        return Err(LauncherError::Io(format!(
            "GitHub releases check failed: HTTP {}",
            resp.status()
        )));
    }
    let release: GithubRelease = resp.json().await.map_err(|e| LauncherError::Io(e.to_string()))?;

    let latest_str = release.tag_name.trim_start_matches('v');
    let latest = Version::parse(latest_str)
        .map_err(|e| LauncherError::Io(format!("bad release tag {:?}: {e}", release.tag_name)))?;

    if latest <= current {
        return Ok(None);
    }

    Ok(Some(LauncherUpdate {
        version: latest_str.to_string(),
        download_url: download_url_for_this_platform()?,
    }))
}

/// The fixed, version-agnostic asset URLs release.yml publishes
/// alongside each release (see its "Also upload under a fixed,
/// version-agnostic filename" step) — always resolves to whatever's
/// currently the latest published release, so this never needs updating
/// when a new version ships.
fn download_url_for_this_platform() -> Result<String, LauncherError> {
    let file = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "securexe-launcher-macos-arm64.dmg",
        ("macos", "x86_64") => "securexe-launcher-macos-x64.dmg",
        ("linux", "x86_64") => "securexe-launcher-linux-amd64.AppImage",
        ("windows", "x86_64") => "securexe-launcher-windows-x64-setup.exe",
        (os, arch) => {
            return Err(LauncherError::Io(format!(
                "no published build for {os}/{arch}"
            )))
        }
    };
    Ok(format!(
        "https://github.com/DavidMarsanic/securexe-launcher/releases/latest/download/{file}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs against the real, live GitHub Releases API for this actual
    /// repo (which has real published releases up to v0.1.15) rather than
    /// a mock — the thing actually worth verifying here is the real
    /// tag-name parsing and semver comparison against real-world data,
    /// not a synthetic fixture that could quietly drift from GitHub's
    /// actual response shape.
    #[tokio::test]
    async fn reports_update_only_when_genuinely_behind() {
        let behind = check("0.1.0").await.expect("check failed");
        assert!(
            behind.is_some(),
            "0.1.0 should be reported behind the real latest release"
        );
        let update = behind.unwrap();
        assert!(update.download_url.starts_with(
            "https://github.com/DavidMarsanic/securexe-launcher/releases/latest/download/"
        ));

        let ahead = check("999.0.0").await.expect("check failed");
        assert!(ahead.is_none(), "a version ahead of any real release should not report an update");
    }
}
