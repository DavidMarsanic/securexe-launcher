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

/// Whether `path` opens its own OS window once run, so we know whether it
/// needs a terminal wrapped around it to be visible/usable at all. Each
/// platform has its own simple, deterministic signal for this — never a
/// heuristic like "wait and see if a window appeared".
#[cfg(target_os = "macos")]
pub(crate) fn is_gui(path: &Path) -> bool {
    // `bundle::resolve_launchable` only produces paths that reach inside a
    // `.app` bundle (`.../Name.app/Contents/MacOS/...`) when the artifact
    // was a bundled GUI app; a plain downloaded binary never has an `.app`
    // path component.
    path.components()
        .any(|c| c.as_os_str().to_string_lossy().ends_with(".app"))
}

/// Windows declares GUI vs console at link time in the PE header's
/// `Subsystem` field — a fixed byte offset, not a guess. Any read/parse
/// failure falls back to `false` (treat as console), which just means we
/// wrap it in a terminal unnecessarily rather than losing output.
#[cfg(target_os = "windows")]
pub(crate) fn is_gui(path: &Path) -> bool {
    const IMAGE_SUBSYSTEM_WINDOWS_GUI: u16 = 2;

    fn read_subsystem(path: &Path) -> std::io::Result<u16> {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(path)?;

        let mut buf4 = [0u8; 4];
        f.seek(SeekFrom::Start(0x3c))?;
        f.read_exact(&mut buf4)?;
        let pe_offset = u32::from_le_bytes(buf4) as u64;

        // PE signature (4 bytes) + IMAGE_FILE_HEADER (20 bytes) = 24, then
        // Subsystem sits at offset 68 into IMAGE_OPTIONAL_HEADER in both
        // the 32-bit and 64-bit layouts (they diverge earlier but
        // re-align by this point).
        f.seek(SeekFrom::Start(pe_offset + 24 + 68))?;
        let mut buf2 = [0u8; 2];
        f.read_exact(&mut buf2)?;
        Ok(u16::from_le_bytes(buf2))
    }

    read_subsystem(path)
        .map(|s| s == IMAGE_SUBSYSTEM_WINDOWS_GUI)
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
pub(crate) fn is_gui(_path: &Path) -> bool {
    // No standard equivalent signal in a bare ELF, and nothing in the
    // catalog currently ships a windowed Linux build — always wrap in a
    // terminal until that changes.
    false
}

#[cfg(target_os = "macos")]
fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn launch_in_terminal(path: &Path, cwd: &Path) -> Result<(), LauncherError> {
    // `open -a Terminal` always starts its new window's shell in $HOME,
    // ignoring our own process's cwd entirely — so setting .current_dir()
    // on this Command wouldn't do anything. A tiny wrapper script that
    // `cd`s into the sandbox before exec'ing the real binary is the only
    // way to control where the launched tool actually runs.
    let wrapper = std::env::temp_dir().join(format!(
        "securexe-launch-{}.sh",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("app")
    ));
    let script = format!(
        "#!/bin/sh\ncd {} || exit 1\nexec {}\n",
        shell_quote(cwd),
        shell_quote(path)
    );
    std::fs::write(&wrapper, script)?;
    make_executable(&wrapper)?;

    Command::new("open").args(["-a", "Terminal"]).arg(&wrapper).spawn()?;

    // Terminal.app has opened and started executing the script within a
    // couple seconds; Unix allows unlinking a file a process still has
    // open (the inode stays alive via that open handle), so this can't
    // disrupt the running command — it just stops these from silently
    // accumulating in the temp folder forever.
    let cleanup_path = wrapper.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(5));
        let _ = std::fs::remove_file(&cleanup_path);
    });

    Ok(())
}

#[cfg(target_os = "windows")]
fn launch_in_terminal(path: &Path, cwd: &Path) -> Result<(), LauncherError> {
    // Unlike Terminal.app, `start` inherits its own initial directory from
    // the cmd process that invokes it — so setting current_dir() here is
    // enough to carry through to the new console window.
    Command::new("cmd")
        .current_dir(cwd)
        // The empty "" is a required placeholder for `start`'s window-title
        // argument — without it, a quoted path is misread as the title.
        .args(["/C", "start", ""])
        .arg(path)
        .spawn()?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn launch_in_terminal(path: &Path, cwd: &Path) -> Result<(), LauncherError> {
    for terminal in ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"] {
        if Command::new(terminal).current_dir(cwd).arg("-e").arg(path).spawn().is_ok() {
            return Ok(());
        }
    }
    // No known terminal emulator available — fall back to a bare spawn
    // rather than failing outright; output just won't be visible.
    Command::new(path).current_dir(cwd).spawn()?;
    Ok(())
}

/// Launches the downloaded, checksum-verified (and, if it was a bundle,
/// already-unzipped) executable — never through a shell, so nothing in the
/// repo/commit/filename can be interpreted as shell syntax. GUI apps open
/// their own window as normal; anything else gets wrapped in a terminal so
/// it's actually visible instead of running invisibly in the background.
/// `cwd` is always the app's own sandbox directory, never the user's real
/// home folder or wherever the launcher process itself happened to start —
/// a tool that reads or writes "the current directory" should only ever
/// touch its own sandboxed space.
pub fn launch(path: &Path, cwd: &Path) -> Result<(), LauncherError> {
    make_executable(path)?;
    if is_gui(path) {
        Command::new(path).current_dir(cwd).spawn()?;
    } else {
        launch_in_terminal(path, cwd)?;
    }
    Ok(())
}
