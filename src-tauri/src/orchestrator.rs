use futures_util::StreamExt;
use serde::Deserialize;
use std::path::Path;
use tokio::io::AsyncWriteExt;

use crate::error::LauncherError;

/// Pinned host the helper is allowed to talk to. The `securexe://` scheme
/// itself never carries a URL/host (see repo.rs) — this is the one place
/// that decides where downloads actually come from.
const ORCHESTRATOR_BASE: &str = "https://worker.brightencode.com";

#[derive(Debug, Deserialize)]
pub struct Artifact {
    pub os: String,
    pub arch: String,
    pub file: Option<String>,
    pub sha256: Option<String>,
    #[serde(default)]
    pub success: bool,
}

#[derive(Debug, Deserialize)]
pub struct Source {
    pub commit: String,
}

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub source: Option<Source>,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
}

impl Manifest {
    pub fn artifact_for(&self, target: &str) -> Option<&Artifact> {
        self.artifacts.iter().find(|a| {
            a.success && format!("{}-{}", a.os, a.arch) == target
        })
    }
}

pub async fn fetch_manifest(
    client: &reqwest::Client,
    slug: &str,
    commit: Option<&str>,
) -> Result<Manifest, LauncherError> {
    let mut url = format!(
        "{ORCHESTRATOR_BASE}/manifest?repo={}",
        urlencoding_component(slug)
    );
    if let Some(c) = commit {
        url.push_str(&format!("&commit={}", urlencoding_component(c)));
    }

    let resp = client.get(&url).send().await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(LauncherError::NotFound(format!("no manifest for {slug}")));
    }
    if !resp.status().is_success() {
        return Err(LauncherError::Network(format!(
            "manifest request failed: {}",
            resp.status()
        )));
    }

    resp.json::<Manifest>()
        .await
        .map_err(|e| LauncherError::Network(format!("bad manifest response: {e}")))
}

/// Streams the artifact to `dest_path`, overwriting any existing file there.
pub async fn download_to(
    client: &reqwest::Client,
    slug: &str,
    target: &str,
    commit: Option<&str>,
    dest_path: &Path,
) -> Result<(), LauncherError> {
    let mut url = format!(
        "{ORCHESTRATOR_BASE}/download?repo={}&target={}",
        urlencoding_component(slug),
        urlencoding_component(target)
    );
    if let Some(c) = commit {
        url.push_str(&format!("&commit={}", urlencoding_component(c)));
    }

    let resp = client.get(&url).send().await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(LauncherError::NotFound(format!(
            "no build for {slug} / {target}"
        )));
    }
    if !resp.status().is_success() {
        return Err(LauncherError::Network(format!(
            "download failed: {}",
            resp.status()
        )));
    }

    if let Some(parent) = dest_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut file = tokio::fs::File::create(dest_path).await?;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(LauncherError::from)?;
        file.write_all(&chunk).await?;
    }
    file.flush().await?;

    Ok(())
}

fn urlencoding_component(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}
