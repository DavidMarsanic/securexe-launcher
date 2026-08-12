mod bundle;
mod error;
mod flow;
mod install;
mod library;
mod orchestrator;
mod platform;
mod repo;
mod run;
mod signature;
mod verify;

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Serialize;
use tauri_plugin_deep_link::DeepLinkExt;

/// The gallery-safe view of a `library::LibraryEntry` — the webview never
/// sees raw filesystem paths, only a slug to relaunch by and an icon
/// already inlined as a data URL (no need to stand up an asset-serving
/// scheme just for a handful of small PNGs).
#[derive(Serialize)]
struct GalleryEntry {
    slug: String,
    repo: String,
    is_gui: bool,
    icon_data_url: Option<String>,
    installed_at: u64,
    last_launched_at: Option<u64>,
}

impl From<library::LibraryEntry> for GalleryEntry {
    fn from(e: library::LibraryEntry) -> Self {
        let icon_data_url = e.icon.and_then(|path| std::fs::read(path).ok()).map(|bytes| {
            format!("data:image/png;base64,{}", STANDARD.encode(bytes))
        });
        GalleryEntry {
            slug: e.slug,
            repo: e.repo,
            is_gui: e.is_gui,
            icon_data_url,
            installed_at: e.installed_at,
            last_launched_at: e.last_launched_at,
        }
    }
}

#[tauri::command]
fn list_library() -> Result<Vec<GalleryEntry>, String> {
    library::load()
        .map(|entries| entries.into_iter().map(GalleryEntry::from).collect())
        .map_err(|e| e.to_string())
}

/// Re-runs an already-downloaded, already-checksum-verified app straight
/// from the local cache — deliberately does *not* require a fresh signed
/// link. The signature's job is authorizing what gets downloaded onto the
/// machine in the first place; once that's verified once, replaying it
/// locally doesn't need the site back in the loop.
#[tauri::command]
fn relaunch(slug: String) -> Result<(), String> {
    let entry = library::find(&slug)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{slug} is not installed"))?;
    run::launch(&entry.path).map_err(|e| e.to_string())?;
    let _ = library::touch_last_launched(&slug);
    Ok(())
}

/// Cold starts can deliver the same launch URL through both
/// `get_current()` and the `on_open_url` listener below — a known overlap
/// in how the OS/plugin replay the Apple Event that launched the app.
/// Without this guard, that double delivery races two downloads against
/// the same cache file and corrupts it. A signed link is single-use in
/// practice (the website mints a fresh `exp`/`sig` per click), so an exact
/// string match is enough to tell "redelivered" apart from "clicked again".
fn already_dispatched(url: &str) -> bool {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(HashSet::new()));
    !seen.lock().unwrap().insert(url.to_string())
}

fn dispatch(app: &tauri::AppHandle, url: url::Url) {
    let url = url.to_string();
    if already_dispatched(&url) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        flow::handle_run_url(app, url).await;
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .invoke_handler(tauri::generate_handler![list_library, relaunch])
        .setup(|app| {
            let handle = app.handle().clone();

            // Only needed so `cargo tauri dev` can be exercised without a
            // real installer having registered the scheme with the OS —
            // bundled/installed builds get this from tauri.conf.json's
            // `plugins.deep-link.desktop.schemes` instead.
            #[cfg(debug_assertions)]
            {
                let _ = handle.deep_link().register("securexe");
            }

            // URLs the app was launched with (cold start).
            if let Ok(Some(urls)) = handle.deep_link().get_current() {
                for url in urls {
                    dispatch(&handle, url);
                }
            }

            // URLs delivered while the app is already running.
            let listener_handle = handle.clone();
            handle.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    dispatch(&listener_handle, url);
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
