use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::error::LauncherError;
use crate::{account, bundle, install, library, orchestrator, platform, repo, run, verify};

pub const STATUS_EVENT: &str = "launcher-status";

#[derive(Clone, Serialize)]
#[serde(tag = "step", rename_all = "lowercase")]
pub enum StatusEvent {
    Resolving { repo: String },
    Downloading { repo: String },
    Verifying { repo: String },
    Launching { repo: String },
    Done { repo: String },
    Linked { user: String },
    Error { message: String },
}

fn emit(app: &AppHandle, event: StatusEvent) {
    let _ = app.emit(STATUS_EVENT, event);
}

/// Entry point for every incoming `securexe://run?...` URL, whether it
/// arrived at cold start or while the app was already running.
pub async fn handle_run_url(app: AppHandle, raw_url: String) {
    if let Err(e) = run_inner(&app, &raw_url).await {
        emit(&app, StatusEvent::Error { message: e.to_string() });
    }
}

async fn run_inner(app: &AppHandle, raw_url: &str) -> Result<(), LauncherError> {
    let req = repo::parse_run_url(raw_url)?;
    install_and_launch(app, req.repo_path(), req.slug(), req.commit.clone()).await
}

/// Fetches (if not already cached), verifies, installs, and launches an
/// app — shared by two callers with different trust models:
///
/// - `run_inner`, for a signed `securexe://run` deep link: the signature is
///   what authorizes *installing something onto this machine at all*, since
///   the OS gives the launcher no way to know which website a custom-scheme
///   link was clicked from (see repo.rs).
/// - the in-app "Update" action (`update_slug` in lib.rs), for an app
///   that's already installed: no signature is involved because none is
///   needed — the user already trusted this exact repo once, and this is
///   only ever reachable by right-clicking something already sitting in
///   their own library, not by a webpage. It just re-runs this same
///   fetch/verify/install/launch sequence with `requested_commit: None`,
///   which resolves to whatever the manifest currently reports as latest.
pub async fn install_and_launch(
    app: &AppHandle,
    repo_path: String,
    slug: String,
    requested_commit: Option<String>,
) -> Result<(), LauncherError> {
    emit(app, StatusEvent::Resolving { repo: repo_path.clone() });

    let target = platform::target_key()?;
    let client = reqwest::Client::builder().build()?;

    let manifest = orchestrator::fetch_manifest(&client, &slug, requested_commit.as_deref()).await?;

    let commit = manifest
        .source
        .as_ref()
        .map(|s| s.commit.clone())
        .or(requested_commit)
        .filter(|c| repo::is_safe_commit(c))
        .ok_or_else(|| LauncherError::NotFound(format!("no resolvable commit for {repo_path}")))?;

    let artifact = manifest
        .artifact_for(&target)
        .ok_or_else(|| LauncherError::NotFound(format!("no build for {repo_path} ({target})")))?;

    let file = artifact
        .file
        .clone()
        .ok_or_else(|| LauncherError::NotFound(format!("manifest missing file for {target}")))?;
    let expected_sha256 = artifact
        .sha256
        .clone()
        .ok_or_else(|| LauncherError::NotFound(format!("manifest missing checksum for {target}")))?;

    let dest = install::artifact_path(&slug, &commit, &file)?;

    let cache_hit = {
        let dest = dest.clone();
        let expected = expected_sha256.clone();
        tokio::task::spawn_blocking(move || install::is_cached(&dest, &expected))
            .await
            .unwrap_or(false)
    };

    if !cache_hit {
        emit(app, StatusEvent::Downloading { repo: repo_path.clone() });
        orchestrator::download_to(&client, &slug, &target, Some(&commit), &dest).await?;

        emit(app, StatusEvent::Verifying { repo: repo_path.clone() });
        let dest_check = dest.clone();
        let actual = tokio::task::spawn_blocking(move || verify::sha256_file(&dest_check))
            .await
            .map_err(|e| LauncherError::Io(e.to_string()))??;

        if !actual.eq_ignore_ascii_case(&expected_sha256) {
            let _ = std::fs::remove_file(&dest);
            return Err(LauncherError::ChecksumMismatch);
        }
    }

    emit(app, StatusEvent::Launching { repo: repo_path.clone() });
    let resolved = {
        let dest = dest.clone();
        tokio::task::spawn_blocking(move || bundle::resolve_launchable(&dest))
            .await
            .map_err(|e| LauncherError::Io(e.to_string()))??
    };

    let is_gui = run::is_gui(&resolved.executable);
    let icon = if let Some(app_bundle) = &resolved.app_bundle {
        let dest_png = install::icon_path(&slug)?;
        let app_bundle = app_bundle.clone();
        tokio::task::spawn_blocking(move || bundle::extract_icon(&app_bundle, &dest_png))
            .await
            .unwrap_or(None)
    } else {
        None
    };
    let existing_entry = library::find(&slug)?;
    let previous_commit =
        library::record_install(&slug, &repo_path, &commit, resolved.executable.clone(), is_gui, icon)?;
    if let Some(old_commit) = previous_commit {
        let _ = install::remove_commit(&slug, &old_commit);
    }

    // Only report "installed" on a genuinely new install or a version
    // change — record_install runs on every launch (including cache hits),
    // and reporting an event on every relaunch of an unchanged app would
    // spam the worker for no reason the website's library view cares about.
    let changed = existing_entry.map(|e| e.commit != commit).unwrap_or(true);
    if changed {
        if let Some(acct) = account::load().ok().flatten() {
            let client = client.clone();
            let repo_path = repo_path.clone();
            let commit = commit.clone();
            tauri::async_runtime::spawn(async move {
                let _ = orchestrator::report_library_event(
                    &client,
                    &acct.device_token,
                    &repo_path,
                    Some(&commit),
                    "installed",
                )
                .await;
            });
        }
    }

    let cwd = install::sandbox_dir(&slug)?;
    run::launch(&resolved.executable, &cwd)?;

    emit(app, StatusEvent::Done { repo: repo_path });
    Ok(())
}

/// Entry point for every incoming `securexe://link?...` URL — associates
/// this device with a Securexe account. The website mints these
/// (signDeviceLinkToken in lib/signing.ts) using the same keypair as
/// `securexe://run` links, just a distinct message shape.
pub async fn handle_link_url(app: AppHandle, raw_url: String) {
    if let Err(e) = link_inner(&app, &raw_url).await {
        emit(&app, StatusEvent::Error { message: e.to_string() });
    }
}

async fn link_inner(app: &AppHandle, raw_url: &str) -> Result<(), LauncherError> {
    let req = repo::parse_link_url(raw_url)?;
    let client = reqwest::Client::builder().build()?;
    let device_id = account::device_id()?;

    let device_token =
        orchestrator::exchange_device_link(&client, &req.user, &req.exp, &req.sig, &device_id)
            .await?;
    account::record_link(req.user.clone(), device_token.clone())?;

    emit(app, StatusEvent::Linked { user: req.user });

    // Backfill: report everything already in the local library, not just
    // installs/uninstalls from this point forward — otherwise anything
    // installed before this device got linked would never be visible on
    // the website, even though it's genuinely present on disk. Re-linking
    // an already-linked device just re-reports the same "installed" state,
    // which is a harmless no-op upsert on the worker's side.
    if let Ok(entries) = library::load() {
        tauri::async_runtime::spawn(async move {
            for entry in entries {
                let _ = orchestrator::report_library_event(
                    &client,
                    &device_token,
                    &entry.repo,
                    Some(&entry.commit),
                    "installed",
                )
                .await;
            }
        });
    }

    Ok(())
}
