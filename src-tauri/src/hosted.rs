//! Prototype: instead of an installed applet spawning its own Chrome
//! `--app=` window (which shows Chrome's own icon in the Dock, not the
//! applet's — a hard limitation of that approach, not fixable by anything
//! in the applet's own bundle), the launcher spawns the applet's local
//! server directly and hosts its UI in a real Tauri window of its own.
//! That window belongs to securexe-launcher's own process, so it shares
//! the launcher's Dock icon rather than Chrome's — and macOS's own
//! right-click-Dock-icon window list (built in, no extra code) becomes
//! the "what's open" affordance, keyed off each window's title.
//!
//! This module is deliberately standalone and not yet wired into the real
//! install/manifest flow (flow.rs) — it's a proof of the mechanism first,
//! on one real app (gif-maker), before deciding whether/how to fold it
//! into the production launch path.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::error::LauncherError;

/// How long to wait for the child process to print its ready URL before
/// giving up — generous, since a cold start (first run, port binding,
/// etc.) can take a moment, but bounded so a genuinely broken child
/// doesn't hang the launcher forever.
const READY_TIMEOUT: Duration = Duration::from_secs(15);

/// Spawns `path` as a hosted applet and opens (or focuses, if already
/// running) a native window titled `title` pointed at whatever local URL
/// it reports on stderr. `label` must be a stable, unique identifier for
/// this applet (e.g. its slug) — it's both the Tauri window label and
/// what a second launch checks to avoid starting a duplicate process.
pub async fn launch_hosted(
    app: &AppHandle,
    label: &str,
    title: &str,
    path: &Path,
    cwd: &Path,
) -> Result<(), LauncherError> {
    // Already running: bring the existing window forward instead of
    // spawning a second process on top of it (which would also just
    // fail outright — the first instance already holds the port).
    if let Some(window) = app.get_webview_window(label) {
        window
            .set_focus()
            .map_err(|e| LauncherError::Io(e.to_string()))?;
        return Ok(());
    }

    let mut child = Command::new(path)
        .current_dir(cwd)
        .env("SECUREXE_HOSTED", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| LauncherError::Io("no stderr pipe on hosted child".into()))?;

    // The child's own startup line ("<Name> running at http://127.0.0.1:PORT
    // — press Ctrl+C to quit") is the one place it reports the port it
    // actually bound — it always asks for port 0 (any free port), so
    // there's no fixed port to assume here; this has to be discovered,
    // not guessed. Once the process exits (crash, missing dependency
    // before it ever got that far), stop waiting rather than hang.
    let url = tokio::time::timeout(READY_TIMEOUT, find_ready_url(stderr))
        .await
        .map_err(|_| LauncherError::NotFound(format!("{title} never reported a ready URL")))??;

    // The child keeps running in the background for as long as this
    // window is open — nothing here waits on it, and its own idle-timeout
    // watchdog is what eventually exits it if the window gets closed
    // without a clean shutdown signal reaching it. Detaching (rather than
    // holding a Child handle somewhere) is fine for this prototype; the
    // real integration will need to track this alongside the window's
    // close event so a closed window actually ends the process.
    tokio::spawn(async move {
        let _ = child.wait().await;
    });

    WebviewWindowBuilder::new(app, label, WebviewUrl::External(url.parse().map_err(
        |e: url::ParseError| LauncherError::Io(format!("bad hosted URL: {e}")),
    )?))
    .title(title)
    .inner_size(900.0, 640.0)
    .build()
    .map_err(|e| LauncherError::Io(e.to_string()))?;

    Ok(())
}

async fn find_ready_url(stderr: tokio::process::ChildStderr) -> Result<String, LauncherError> {
    let mut lines = BufReader::new(stderr).lines();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|e| LauncherError::Io(e.to_string()))?
    {
        if let Some(start) = line.find("http://") {
            let rest = &line[start..];
            let url = rest.split_whitespace().next().unwrap_or(rest);
            return Ok(url.to_string());
        }
    }
    Err(LauncherError::NotFound(
        "hosted process exited before reporting a ready URL".into(),
    ))
}
