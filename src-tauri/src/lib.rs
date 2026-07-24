mod bundle;
mod error;
mod flow;
mod install;
mod orchestrator;
mod platform;
mod repo;
mod run;
mod signature;
mod verify;

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use tauri_plugin_deep_link::DeepLinkExt;

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
