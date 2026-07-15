use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::error::LauncherError;
use crate::{install, orchestrator, platform, repo, run, verify};

pub const STATUS_EVENT: &str = "launcher-status";

#[derive(Clone, Serialize)]
#[serde(tag = "step", rename_all = "lowercase")]
pub enum StatusEvent {
    Resolving { repo: String },
    Downloading { repo: String },
    Verifying { repo: String },
    Launching { repo: String },
    Done { repo: String },
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
    let repo_path = req.repo_path();
    emit(app, StatusEvent::Resolving { repo: repo_path.clone() });

    let target = platform::target_key()?;
    let slug = req.slug();
    let client = reqwest::Client::builder().build()?;

    let manifest = orchestrator::fetch_manifest(&client, &slug, req.commit.as_deref()).await?;

    let commit = manifest
        .source
        .as_ref()
        .map(|s| s.commit.clone())
        .or_else(|| req.commit.clone())
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
    run::launch(&dest)?;

    emit(app, StatusEvent::Done { repo: repo_path });
    Ok(())
}
