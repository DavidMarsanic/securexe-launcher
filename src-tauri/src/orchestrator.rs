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

#[derive(Debug, Deserialize)]
struct DeviceLinkResponse {
    status: String,
    #[serde(rename = "deviceToken")]
    device_token: Option<String>,
}

/// Trades a signature-verified `securexe://link` token for a durable
/// per-device bearer token. The worker is expected to independently
/// re-verify the Ed25519 signature itself (it needs the same public key
/// baked into signature.rs, which is fine to share since it's a public
/// key) rather than trust that this binary already checked it — a modified
/// client could otherwise call this endpoint directly with a forged
/// payload and no signature check would ever run.
///
/// NOTE: `POST /devices/link` doesn't exist on the worker yet — this is a
/// guessed contract (same seam as securexe-web's `/api/claim` calling a
/// not-yet-built `/trusted-owners`). Expect this to fail until the worker
/// side ships a matching endpoint.
pub async fn exchange_device_link(
    client: &reqwest::Client,
    user: &str,
    exp: &str,
    sig: &str,
    device_id: &str,
) -> Result<String, LauncherError> {
    let resp = client
        .post(format!("{ORCHESTRATOR_BASE}/devices/link"))
        .json(&serde_json::json!({
            "user": user,
            "exp": exp,
            "sig": sig,
            "deviceId": device_id,
        }))
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(LauncherError::Network(format!(
            "device link exchange failed: {}",
            resp.status()
        )));
    }

    let parsed: DeviceLinkResponse = resp
        .json()
        .await
        .map_err(|e| LauncherError::Network(format!("bad device-link response: {e}")))?;

    if parsed.status != "ok" {
        return Err(LauncherError::Network(
            "unexpected device-link response".into(),
        ));
    }
    parsed
        .device_token
        .ok_or_else(|| LauncherError::Network("device-link response missing token".into()))
}

/// Best-effort install/uninstall event report, used to keep the website's
/// "your library" view in sync with what's actually on this device. Callers
/// (flow.rs, lib.rs's uninstall command) fire this on a spawned task and
/// discard the result — reporting failing should never block the local
/// install/uninstall it's describing. Same not-yet-built-on-the-worker
/// caveat as exchange_device_link.
pub async fn report_library_event(
    client: &reqwest::Client,
    device_token: &str,
    repo: &str,
    commit: Option<&str>,
    action: &str,
) -> Result<(), LauncherError> {
    let resp = client
        .post(format!("{ORCHESTRATOR_BASE}/library/events"))
        .bearer_auth(device_token)
        .json(&serde_json::json!({
            "repo": repo,
            "commit": commit,
            "action": action,
        }))
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(LauncherError::Network(format!(
            "library event report failed: {}",
            resp.status()
        )));
    }
    Ok(())
}
