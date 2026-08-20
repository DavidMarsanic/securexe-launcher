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

/// Downloads the latest release, replaces the installed app in
/// `/Applications`, strips the quarantine flag Gatekeeper would
/// otherwise block on, and relaunches — all visibly, in a Terminal
/// window, rather than silently in the background.
///
/// This is deliberately not a "real" auto-updater: the app is currently
/// ad-hoc signed (no paid Apple Developer ID), and ad-hoc signatures have
/// no stable identity across builds — macOS can't tell that a rebuilt
/// version is "the same app", so TCC permission grants (Local Network,
/// etc.) reset on every update regardless of how the new build gets
/// installed. That part isn't fixable without a real Developer ID +
/// notarization. What this *does* fix is the redundant manual toil on
/// top of that: no more downloading a DMG, mounting it, dragging the
/// app over, ejecting, and relaunching by hand — and no Gatekeeper
/// "unidentified developer" prompt either, since stripping quarantine
/// here is exactly the standard workaround for that (see the codesign
/// conversation this came out of).
#[cfg(target_os = "macos")]
fn build_update_script(dmg_url: &str) -> String {
    let app_name = "Brightencode.app";
    // Matches the Cargo package/bin name (no [[bin]] override in
    // Cargo.toml, so the binary defaults to the package name) — this is
    // the actual process name macOS shows for `pkill -x`, not the
    // human-readable bundle display name above.
    let process_name = "securexe-launcher";

    format!(
        r#"#!/bin/sh
set -e
echo "Updating Brightencode..."
TMP_DMG=$(mktemp -t securexe-launcher-update).dmg
curl -fL "{dmg_url}" -o "$TMP_DMG"

echo "Quitting the running app..."
pkill -x {process_name} 2>/dev/null || true
sleep 1

echo "Mounting update..."
MOUNT_POINT=$(hdiutil attach "$TMP_DMG" -nobrowse -readonly | tail -1 | awk -F'\t' '{{print $NF}}')

echo "Installing to /Applications..."
rm -rf "/Applications/{app_name}"
cp -R "$MOUNT_POINT/{app_name}" "/Applications/"

echo "Cleaning up..."
hdiutil detach "$MOUNT_POINT" -quiet
rm -f "$TMP_DMG"

echo "Clearing the quarantine flag so Gatekeeper doesn't block the relaunch..."
xattr -dr com.apple.quarantine "/Applications/{app_name}"

echo "Relaunching..."
open "/Applications/{app_name}"
echo "Done — this window will close in a moment."

# Close this Terminal window on its own instead of leaving the user to do
# it by hand. Targeted by controlling tty (unique to this window, so this
# is safe with other Terminal windows open) and fired from a disowned
# background subshell after a short delay so this script has already
# exited by the time it runs — Terminal only shows a "process still
# running" confirmation when asked to close a window that still has one,
# so letting our own process end first avoids that prompt entirely.
THIS_TTY=$(tty 2>/dev/null || true)
if [ -n "$THIS_TTY" ]; then
  (sleep 1; osascript -e "tell application \"Terminal\" to close (first window whose tty is \"$THIS_TTY\")" >/dev/null 2>&1) &
  disown
fi
"#,
    )
}

#[cfg(target_os = "macos")]
pub fn update_via_terminal() -> Result<(), LauncherError> {
    let dmg_url = download_url_for_this_platform()?;
    let script = build_update_script(&dmg_url);

    let wrapper = std::env::temp_dir().join("securexe-launcher-update.sh");
    std::fs::write(&wrapper, script)?;
    let mut perms = std::fs::metadata(&wrapper)?.permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(&wrapper, perms)?;

    std::process::Command::new("open")
        .args(["-a", "Terminal"])
        .arg(&wrapper)
        .spawn()?;

    Ok(())
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

    /// Checks the generated update script is actually valid shell (`sh -n`
    /// parses it without executing anything) and contains every step in
    /// the right order — real syntax verification, not just "the string
    /// contains the substring 'curl' somewhere". Never runs the script
    /// itself: curl/pkill/hdiutil/cp against real infrastructure has no
    /// business happening in a test.
    #[cfg(target_os = "macos")]
    #[test]
    fn update_script_is_valid_shell_in_correct_order() {
        let script = build_update_script("https://example.com/fake.dmg");

        for step in ["curl", "pkill", "hdiutil attach", "cp -R", "hdiutil detach", "xattr -dr com.apple.quarantine", "open ", "tty", "close (first window"] {
            assert!(script.contains(step), "script is missing step: {step}");
        }
        let order = ["curl", "pkill", "hdiutil attach", "cp -R", "xattr -dr", "open \"", "tty", "close (first window"];
        let positions: Vec<_> = order.iter().map(|s| script.find(s).unwrap_or_else(|| panic!("missing {s}"))).collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]), "steps are out of order: {positions:?}");

        let dir = std::env::temp_dir().join(format!("self-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("script.sh");
        std::fs::write(&path, &script).unwrap();

        let output = std::process::Command::new("sh").arg("-n").arg(&path).output().unwrap();
        assert!(
            output.status.success(),
            "generated script has a shell syntax error: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
