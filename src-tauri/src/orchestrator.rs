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

/// One catalog entry from `GET /search` — the Browse tab's data source.
/// `icon_url` is already an absolute, ready-to-use URL (the worker's own
/// generated-badge-or-owner-uploaded-icon precedence, same as securexe-web's
/// catalog); unlike the installed gallery's `icon_data_url`, nothing here
/// fetches or caches it locally, the `<img>` tag just points straight at it.
#[derive(Debug, Deserialize)]
pub struct SearchResult {
    pub repo: String,
    #[serde(rename = "iconUrl")]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    pub artifacts: Vec<Artifact>,
}

impl SearchResult {
    pub fn artifact_for(&self, target: &str) -> Option<&Artifact> {
        self.artifacts.iter().find(|a| {
            a.success && format!("{}-{}", a.os, a.arch) == target
        })
    }
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    status: String,
    #[serde(default)]
    results: Vec<SearchResult>,
}

/// Fetches the public catalog (or a filtered slice of it) from the worker's
/// `/search` endpoint — no auth, this is the same live data securexe-web's
/// own Library page and homepage already read. `query`, when present, is
/// forwarded as-is; the worker does the matching (repo slug, GitHub name,
/// toolchain version, per-artifact fields) — this is not a client-side
/// filter over a locally cached full catalog.
pub async fn search_catalog(
    client: &reqwest::Client,
    query: Option<&str>,
) -> Result<Vec<SearchResult>, LauncherError> {
    let url = format!("{ORCHESTRATOR_BASE}/search?q={}", urlencoding_component(query.unwrap_or("")));

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(LauncherError::Network(format!(
            "search request failed: {}",
            resp.status()
        )));
    }

    let parsed: SearchResponse = resp
        .json()
        .await
        .map_err(|e| LauncherError::Network(format!("bad search response: {e}")))?;
    if parsed.status != "ok" {
        return Err(LauncherError::Network("unexpected search response".into()));
    }
    Ok(parsed.results)
}

/// One installed app with a newer build available, as reported by the
/// worker itself rather than computed here — see `fetch_updates`.
#[derive(Debug, Deserialize)]
pub struct UpdateInfo {
    pub repo: String,
}

#[derive(Debug, Deserialize)]
struct UpdatesResponse {
    status: String,
    #[serde(default)]
    updates: Vec<UpdateInfo>,
}

/// Asks the worker which of this device's installed apps are stale, in one
/// call, instead of fetching every app's manifest and comparing commits
/// client-side. The worker already tracks per-device install state (via
/// `report_install_event`) and already has to compute "what's the latest
/// build" for `/download` — this reuses that same logic server-side, so
/// "there's an update" and "what a download actually serves" can't
/// disagree the way two independent implementations could.
pub async fn fetch_updates(
    client: &reqwest::Client,
    device_token: &str,
) -> Result<Vec<UpdateInfo>, LauncherError> {
    let resp = client
        .get(format!("{ORCHESTRATOR_BASE}/updates"))
        .bearer_auth(device_token)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(LauncherError::Network(format!(
            "updates request failed: {}",
            resp.status()
        )));
    }

    let parsed: UpdatesResponse = resp
        .json()
        .await
        .map_err(|e| LauncherError::Network(format!("bad updates response: {e}")))?;
    if parsed.status != "ok" {
        return Err(LauncherError::Network("unexpected updates response".into()));
    }
    Ok(parsed.updates)
}

/// The worker's icon URL for `repo` — same endpoint `fetch_icon_svg` below
/// downloads, but as a plain URL string for callers (My Apps' library
/// section) that just want the webview to load it directly via `<img>`,
/// the same way Browse tiles already use `SearchResult::icon_url` without
/// an extra fetch/cache round-trip.
pub fn icon_url(repo: &str) -> String {
    format!("{ORCHESTRATOR_BASE}/icon?repo={}", urlencoding_component(repo))
}

/// One record from the worker's account-wide library — `GET /library`.
/// As of the backend's library/install split, this is a genuinely separate,
/// persistent, per-account *ownership* concept (`LIBRARY_DIR`, keyed by
/// GitHub username) — nothing to do with what's installed where. There's
/// no auto-add on install and no per-device shape anymore (no `deviceId`,
/// no cross-device dedup needed): one row per repo the account owns.
#[derive(Debug, Deserialize)]
pub struct LibraryItem {
    pub repo: String,
}

#[derive(Debug, Deserialize)]
struct LibraryResponse {
    #[serde(default)]
    items: Vec<LibraryItem>,
}

/// Fetches this account's library — everything explicitly added to it,
/// independent of whether it's installed anywhere. Bearer-authed with the
/// device token; written against the intended contract, so if the worker
/// hasn't rolled this split out yet (or doesn't accept device tokens here),
/// this surfaces that as a real `Err` rather than pretending the library is
/// just empty.
pub async fn fetch_account_library(
    client: &reqwest::Client,
    device_token: &str,
) -> Result<Vec<LibraryItem>, LauncherError> {
    let resp = client
        .get(format!("{ORCHESTRATOR_BASE}/library"))
        .bearer_auth(device_token)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(LauncherError::Network(format!(
            "library request failed: {}",
            resp.status()
        )));
    }

    let parsed: LibraryResponse = resp
        .json()
        .await
        .map_err(|e| LauncherError::Network(format!("bad library response: {e}")))?;
    Ok(parsed.items)
}

/// Removes `repo` from this account's library — `POST /library/remove`.
/// A single ownership record, no cascade to any device's install state
/// (removing something from your library doesn't uninstall it anywhere) —
/// this is the My Apps library section's "Remove" action.
pub async fn remove_from_account_library(
    client: &reqwest::Client,
    device_token: &str,
    repo: &str,
) -> Result<(), LauncherError> {
    let resp = client
        .post(format!("{ORCHESTRATOR_BASE}/library/remove"))
        .bearer_auth(device_token)
        .json(&serde_json::json!({ "repo": repo }))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(LauncherError::Network(format!(
            "library remove failed: {}",
            resp.status()
        )));
    }
    Ok(())
}

/// Fetches the worker's generated icon for `repo` ("owner/repo") — an
/// always-available SVG badge (deterministic gradient + initials, and a
/// real uploaded icon instead when the worker has one on record) served at
/// `GET /icon?repo=<repo>` regardless of whether the repo ships its own
/// `.icns`. This is the exact same `iconUrl` securexe-web's catalog uses
/// (see its `RepoIcon` component) — fetching it here instead of generating
/// our own fallback badge keeps the launcher and the website showing
/// identical icons for the same app, rather than two different guesses at
/// the same thing.
pub async fn fetch_icon_svg(client: &reqwest::Client, repo: &str) -> Result<Vec<u8>, LauncherError> {
    let url = icon_url(repo);
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(LauncherError::Network(format!(
            "icon request failed: {}",
            resp.status()
        )));
    }
    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| LauncherError::Network(format!("bad icon response: {e}")))
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

/// Best-effort install/uninstall/launch event report — per-device and
/// ephemeral (`INSTALLS_DIR` on the worker), completely separate from the
/// account-wide library above. `action` is one of "installed" / "uninstalled"
/// / "launched" only; there's no "removed" here — that meaning now belongs
/// to `remove_from_account_library`. Callers (flow.rs, lib.rs's uninstall
/// command) fire this on a spawned task and discard the result — reporting
/// failing should never block the local install/uninstall it's describing.
pub async fn report_install_event(
    client: &reqwest::Client,
    device_token: &str,
    repo: &str,
    commit: Option<&str>,
    action: &str,
) -> Result<(), LauncherError> {
    let resp = client
        .post(format!("{ORCHESTRATOR_BASE}/installs/events"))
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
            "install event report failed: {}",
            resp.status()
        )));
    }
    Ok(())
}

/// Disconnects this device from the account on the worker side — called
/// from the `unlink` command while the device token is still in hand.
/// Without this, unlinking only ever cleared local state, so the website
/// went on listing a device the user had explicitly disconnected.
///
/// NOTE: `POST /devices/unlink` doesn't exist on the worker yet — same
/// not-yet-built seam as exchange_device_link.
pub async fn unlink_device(
    client: &reqwest::Client,
    device_token: &str,
) -> Result<(), LauncherError> {
    let resp = client
        .post(format!("{ORCHESTRATOR_BASE}/devices/unlink"))
        .bearer_auth(device_token)
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(LauncherError::Network(format!(
            "device unlink failed: {}",
            resp.status()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs against the real, live worker (same convention as
    /// self_update's tests hitting the real GitHub Releases API) rather
    /// than a mock — the thing actually worth verifying is that this repo
    /// really does get back a renderable SVG icon from the real endpoint,
    /// not just that our own request-building code compiles.
    #[tokio::test]
    async fn fetch_icon_svg_returns_a_real_svg() {
        let client = reqwest::Client::new();
        let bytes = fetch_icon_svg(&client, "DavidMarsanic/pdf-toolkit")
            .await
            .expect("fetch_icon_svg failed");

        let svg = String::from_utf8(bytes).expect("icon response wasn't valid UTF-8");
        assert!(svg.contains("<svg"), "response doesn't look like an SVG: {svg}");
        assert!(svg.contains("</svg>"), "response doesn't look like a complete SVG: {svg}");
    }
}
