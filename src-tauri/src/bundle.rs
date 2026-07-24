use std::path::{Path, PathBuf};

use crate::error::LauncherError;

/// Some artifacts (macOS `.app.zip` builds) are a zipped app bundle rather
/// than a directly executable file — a bundle's `CFBundleExecutable` is what
/// actually needs to run, resolved from its `Info.plist`, not the bundle
/// directory itself. Anything else (a plain binary) passes through
/// unchanged.
pub fn resolve_launchable(path: &Path) -> Result<PathBuf, LauncherError> {
    if path.extension().and_then(|e| e.to_str()) != Some("zip") {
        return Ok(path.to_path_buf());
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

    let executable = bundle_executable_name(&app_dir)?;
    Ok(app_dir.join("Contents").join("MacOS").join(executable))
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

fn bundle_executable_name(app_dir: &Path) -> Result<String, LauncherError> {
    let plist_path = app_dir.join("Contents").join("Info.plist");
    let value = plist::Value::from_file(&plist_path)
        .map_err(|e| LauncherError::Io(format!("bad Info.plist: {e}")))?;
    value
        .as_dictionary()
        .and_then(|d| d.get("CFBundleExecutable"))
        .and_then(|v| v.as_string())
        .map(|s| s.to_string())
        .ok_or_else(|| LauncherError::Io("Info.plist missing CFBundleExecutable".into()))
}
