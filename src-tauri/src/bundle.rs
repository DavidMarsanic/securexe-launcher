use std::path::{Path, PathBuf};

use crate::error::LauncherError;

/// The actual executable to launch, plus which `.app` bundle (if any) it
/// came from — the bundle dir is what the gallery needs afterward to pull
/// out an icon; nothing else cares about it.
pub struct Resolved {
    pub executable: PathBuf,
    pub app_bundle: Option<PathBuf>,
}

/// Some artifacts (macOS `.app.zip` builds) are a zipped app bundle rather
/// than a directly executable file — a bundle's `CFBundleExecutable` is what
/// actually needs to run, resolved from its `Info.plist`, not the bundle
/// directory itself. Anything else (a plain binary) passes through
/// unchanged.
pub fn resolve_launchable(path: &Path) -> Result<Resolved, LauncherError> {
    if path.extension().and_then(|e| e.to_str()) != Some("zip") {
        return Ok(Resolved {
            executable: path.to_path_buf(),
            app_bundle: None,
        });
    }

    let extract_dir = {
        let mut dir = path.as_os_str().to_owned();
        dir.push(".extracted");
        PathBuf::from(dir)
    };

    let app_dir = match find_app_bundle(&extract_dir)? {
        Some(dir) => dir,
        None => {
            extract_zip(path, &extract_dir)?;
            find_app_bundle(&extract_dir)?.ok_or_else(|| {
                LauncherError::Io(format!("no .app bundle found in {}", path.display()))
            })?
        }
    };

    let executable_name = bundle_executable_name(&app_dir)?;
    Ok(Resolved {
        executable: app_dir.join("Contents").join("MacOS").join(executable_name),
        app_bundle: Some(app_dir),
    })
}

fn extract_zip(zip_path: &Path, dest_dir: &Path) -> Result<(), LauncherError> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| LauncherError::Io(format!("bad archive: {e}")))?;
    archive
        .extract(dest_dir)
        .map_err(|e| LauncherError::Io(format!("extract failed: {e}")))?;
    Ok(())
}

fn find_app_bundle(dir: &Path) -> Result<Option<PathBuf>, LauncherError> {
    if !dir.is_dir() {
        return Ok(None);
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() && path.extension().and_then(|e| e.to_str()) == Some("app") {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn bundle_plist(app_dir: &Path) -> Result<plist::Value, LauncherError> {
    let plist_path = app_dir.join("Contents").join("Info.plist");
    plist::Value::from_file(&plist_path).map_err(|e| LauncherError::Io(format!("bad Info.plist: {e}")))
}

fn bundle_executable_name(app_dir: &Path) -> Result<String, LauncherError> {
    bundle_plist(app_dir)?
        .as_dictionary()
        .and_then(|d| d.get("CFBundleExecutable"))
        .and_then(|v| v.as_string())
        .map(|s| s.to_string())
        .ok_or_else(|| LauncherError::Io("Info.plist missing CFBundleExecutable".into()))
}

/// Converts a bundle's `.icns` icon to a PNG the gallery can display,
/// caching the result at `dest_png`. Best-effort only — an icon is
/// cosmetic, so any failure (no `CFBundleIconFile` key, missing `sips`,
/// non-macOS) just means the gallery falls back to its generic glyph
/// instead of failing the install.
#[cfg(target_os = "macos")]
pub fn extract_icon(app_dir: &Path, dest_png: &Path) -> Option<PathBuf> {
    let icon_file = bundle_plist(app_dir)
        .ok()?
        .as_dictionary()?
        .get("CFBundleIconFile")?
        .as_string()?
        .to_string();
    let icon_file = if icon_file.ends_with(".icns") {
        icon_file
    } else {
        format!("{icon_file}.icns")
    };
    let icns_path = app_dir.join("Contents").join("Resources").join(icon_file);
    if !icns_path.is_file() {
        return None;
    }

    if let Some(parent) = dest_png.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }

    let status = std::process::Command::new("sips")
        .args(["-s", "format", "png"])
        .arg(&icns_path)
        .arg("--out")
        .arg(dest_png)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;

    if status.success() && dest_png.is_file() {
        Some(dest_png.to_path_buf())
    } else {
        None
    }
}

#[cfg(not(target_os = "macos"))]
pub fn extract_icon(_app_dir: &Path, _dest_png: &Path) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression check for a real theory ruled out while debugging a live
    /// hosted-launch failure: hosted.rs spawns the resolved executable
    /// directly (unlike run::launch, which calls make_executable first),
    /// so if extract_zip ever stopped restoring the executable bit a real
    /// macOS .app.zip stores for launch.sh, the spawn would silently fail
    /// and every GUI app would fall back to the old Chrome-window path
    /// with no visible error. Builds its own zip (rather than depending on
    /// a real downloaded artifact on disk) so it runs the same everywhere,
    /// storing a Unix mode via `unix_permissions` the same way a real
    /// build's zip does.
    #[test]
    fn extract_zip_restores_executable_bit() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        use zip::write::SimpleFileOptions;

        let dir = std::env::temp_dir().join(format!("bundle-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("Fixture.app.zip");

        let file = std::fs::File::create(&zip_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default().unix_permissions(0o755);
        writer.start_file("Fixture.app/Contents/MacOS/launch.sh", opts).unwrap();
        writer.write_all(b"#!/bin/sh\necho hi\n").unwrap();
        writer
            .start_file(
                "Fixture.app/Contents/Info.plist",
                SimpleFileOptions::default().unix_permissions(0o644),
            )
            .unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>launch.sh</string>
</dict></plist>"#,
            )
            .unwrap();
        writer.finish().unwrap();

        let resolved = resolve_launchable(&zip_path).expect("resolve_launchable failed");
        let mode = std::fs::metadata(&resolved.executable)
            .expect("stat extracted executable")
            .permissions()
            .mode();
        assert!(
            mode & 0o111 != 0,
            "extracted {} is not executable (mode {:o}) — zip crate's extract() dropped the Unix permission bits the archive stores",
            resolved.executable.display(),
            mode
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
