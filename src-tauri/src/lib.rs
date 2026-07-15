mod error;
mod flow;
mod install;
mod orchestrator;
mod platform;
mod repo;
mod run;
mod signature;
mod verify;

use tauri_plugin_deep_link::DeepLinkExt;

fn dispatch(app: &tauri::AppHandle, url: url::Url) {
    let app = app.clone();
    let url = url.to_string();
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
