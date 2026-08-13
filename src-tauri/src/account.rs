use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::LauncherError;

/// This device's link to a Securexe account. `device_token` is a bearer
/// credential for the worker's library-event endpoints — never sent to the
/// webview (see AccountInfo in lib.rs for the sanitized shape that crosses
/// that boundary), same discipline as LibraryEntry's raw filesystem paths.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub github_username: String,
    pub device_token: String,
    pub linked_at: u64,
}

fn securexe_home() -> Result<PathBuf, LauncherError> {
    let home = dirs::home_dir()
        .ok_or_else(|| LauncherError::Io("could not resolve home directory".into()))?;
    Ok(home.join(".securexe"))
}

fn account_path() -> Result<PathBuf, LauncherError> {
    Ok(securexe_home()?.join("account.json"))
}

pub fn load() -> Result<Option<Account>, LauncherError> {
    let path = account_path()?;
    if !path.is_file() {
        return Ok(None);
    }
    let data = std::fs::read_to_string(&path)?;
    serde_json::from_str(&data)
        .map(Some)
        .map_err(|e| LauncherError::Io(format!("bad account.json: {e}")))
}

pub fn clear() -> Result<(), LauncherError> {
    let path = account_path()?;
    if path.is_file() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn record_link(github_username: String, device_token: String) -> Result<(), LauncherError> {
    let path = account_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let account = Account {
        github_username,
        device_token,
        linked_at: now(),
    };
    let data = serde_json::to_string_pretty(&account)
        .map_err(|e| LauncherError::Io(format!("failed to serialize account: {e}")))?;
    std::fs::write(&path, data)?;
    Ok(())
}

fn device_id_path() -> Result<PathBuf, LauncherError> {
    Ok(securexe_home()?.join("device_id"))
}

/// A random identifier for this physical install of the launcher, generated
/// once on first use and persisted independently of account linking (it
/// survives unlink/relink). Sent to the worker during the link exchange so
/// re-linking the same machine reports the same device rather than the
/// worker seeing a brand-new one every time.
pub fn device_id() -> Result<String, LauncherError> {
    let path = device_id_path()?;
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    let id = hex::encode(rand::random::<[u8; 16]>());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &id)?;
    Ok(id)
}
