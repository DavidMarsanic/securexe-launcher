mod account;
mod bundle;
mod error;
mod flow;
mod hosted;
mod install;
mod library;
mod orchestrator;
mod platform;
mod repo;
mod run;
mod self_update;
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
        let icon_data_url = e.icon.and_then(|path| {
            // Two possible sources land here: a bundle's own `.icns`,
            // always converted to PNG by bundle::extract_icon, or the
            // worker's generated badge, cached as-is at
            // install::worker_icon_path — already an SVG, so no conversion
            // step for that one. The extension is what tells them apart.
            let mime = if path.extension().and_then(|e| e.to_str()) == Some("svg") {
                "image/svg+xml"
            } else {
                "image/png"
            };
            let bytes = std::fs::read(&path).ok()?;
            Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
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

/// One Browse-tab tile — a `/search` result reduced to what the webview
/// needs. Unlike `GalleryEntry`, `icon_url` is a live remote URL the
/// frontend loads directly (no local caching/round-trip), since this is
/// catalog browsing, not an already-installed app.
#[derive(Serialize)]
struct CatalogEntry {
    repo: String,
    slug: String,
    icon_url: Option<String>,
    installed: bool,
    available: bool,
}

/// Lists (optionally filtered by `query`) every applet in the public
/// catalog via the worker's `/search` — the Browse tab's data source. Utility
/// repos (dev tools like icon-composer, not meant for end users to browse)
/// are filtered out, same default as securexe-web's own Library page.
/// `installed` is cross-referenced against the local library so the
/// frontend can show "Install" vs "Launch" per tile; `available` reflects
/// whether the worker actually has a successful build for this platform.
#[tauri::command]
async fn browse_catalog(query: Option<String>) -> Result<Vec<CatalogEntry>, String> {
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;
    let results = orchestrator::search_catalog(&client, query.as_deref())
        .await
        .map_err(|e| e.to_string())?;

    let installed: HashSet<String> = library::load()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|e| e.repo)
        .collect();
    let target = platform::target_key().map_err(|e| e.to_string())?;

    Ok(results
        .into_iter()
        .filter(|r| r.kind.as_deref() != Some("utility"))
        .map(|r| CatalogEntry {
            available: r.artifact_for(&target).is_some(),
            installed: installed.contains(&r.repo),
            slug: r.repo.replacen('/', "__", 1),
            repo: r.repo,
            icon_url: r.icon_url,
        })
        .collect())
}

/// Removes `repo` from the account's library — the My Apps library
/// section's "Remove" action, for an app that isn't installed on *this*
/// device (if it were, `remove_from_library`/`uninstall` above would apply
/// instead, since those act on a local library.json entry). This is My
/// Apps' library-section "Remove" — the account library and per-device
/// installs are now fully separate concepts on the backend
/// (`POST /library/remove`, no cascade to any device's install state), so
/// removing something here never touches what's installed anywhere.
/// There's no local state to update either way — purely a backend call —
/// so unlike `uninstall`/`remove_from_library`'s fire-and-forget spawn,
/// this is awaited: with nothing local to fall back on, the frontend needs
/// to know honestly whether it actually took.
#[tauri::command]
async fn remove_from_account(repo: String) -> Result<(), String> {
    if !repo::is_safe_repo_path(&repo) {
        return Err(format!("invalid repo '{repo}'"));
    }
    let acct = account::load()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no account linked".to_string())?;
    let client = reqwest::Client::new();
    orchestrator::remove_from_account_library(&client, &acct.device_token, &repo)
        .await
        .map_err(|e| e.to_string())
}

/// One entry in My Apps' "library" section — a repo owned by this account
/// (explicitly added, independent of install state) that isn't installed
/// on *this* device.
#[derive(Serialize)]
struct AccountLibraryEntry {
    repo: String,
    slug: String,
    icon_url: String,
}

/// Lists the account's library (`GET /library`) minus whatever's already
/// installed on this device — those show in My Apps' Installed section
/// instead, from purely local data. There's deliberately no auto-add on
/// install (a fresh install never shows up here on its own — the account
/// library only ever grows through an explicit add), so an empty result is
/// entirely expected for an account that's never added anything. Returns an
/// empty list (not an error) when no account is linked, since that's a
/// legitimate "nothing to show" rather than a failure; a real fetch failure
/// (see `orchestrator::fetch_account_library`) is left as `Err` so the
/// frontend can say so honestly instead of silently rendering an empty
/// section that looks the same as "you just haven't added anything yet".
#[tauri::command]
async fn list_account_library() -> Result<Vec<AccountLibraryEntry>, String> {
    let Some(acct) = account::load().map_err(|e| e.to_string())? else {
        return Ok(Vec::new());
    };
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;
    let items = orchestrator::fetch_account_library(&client, &acct.device_token)
        .await
        .map_err(|e| e.to_string())?;

    let installed_locally: HashSet<String> =
        library::load().map_err(|e| e.to_string())?.into_iter().map(|e| e.repo).collect();

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        if installed_locally.contains(&item.repo) || !seen.insert(item.repo.clone()) {
            continue;
        }
        out.push(AccountLibraryEntry {
            slug: item.repo.replacen('/', "__", 1),
            icon_url: orchestrator::icon_url(&item.repo),
            repo: item.repo,
        });
    }
    Ok(out)
}

/// Installs (and launches) a Browse-tab tile — reuses the exact same
/// fetch/verify/install/launch pipeline as a signed `securexe://run` link
/// or the in-app "Update" action (see `flow::install_and_launch`'s doc
/// comment for why no signature is needed here: the user is explicitly
/// clicking Install on something already sitting in their own launcher,
/// not following an arbitrary webpage link).
///
/// Also adds `repo` to the account library — the backend keeps install and
/// library fully decoupled (no auto-add on install server-side), but
/// clicking Install in Browse is a deliberate "I want this app" action, so
/// this is the one client-side path that does both. Best-effort and
/// fire-and-forget, same as the install-event report inside
/// `install_and_launch`: a library-sync hiccup shouldn't surface as
/// "Install failed" when the app itself installed and launched fine.
#[tauri::command]
async fn install_from_catalog(app: tauri::AppHandle, repo: String) -> Result<(), String> {
    if !repo::is_safe_repo_path(&repo) {
        return Err(format!("invalid repo '{repo}'"));
    }
    let slug = repo.replacen('/', "__", 1);
    flow::install_and_launch(&app, repo.clone(), slug, None, true)
        .await
        .map_err(|e| e.to_string())?;

    if let Some(acct) = account::load().ok().flatten() {
        tauri::async_runtime::spawn(async move {
            let client = reqwest::Client::new();
            let _ = orchestrator::add_to_library(&client, &acct.device_token, &repo).await;
        });
    }
    Ok(())
}

/// One app whose icon was just fetched from the worker and cached — all
/// `backfill_icons` returns, since the frontend only needs to patch the
/// tiles that actually changed, not a full re-describe of the library.
#[derive(Serialize)]
struct BackfilledIcon {
    slug: String,
    icon_data_url: String,
}

/// Re-syncs every installed app's icon against the worker — the worker is
/// the authoritative source (same iconUrl-or-generated-badge securexe-web's
/// RepoIcon shows), so this re-fetches unconditionally rather than only
/// filling in apps with nothing cached: an app installed before
/// `flow::install_and_launch` preferred the worker icon may already have a
/// generic bundle-extracted one cached, and that's exactly what needs
/// correcting, not just genuinely missing icons. Called once after the
/// gallery's initial paint from `list_library` (see main.js), not folded
/// into that command itself, so a slow/offline network never delays
/// getting tiles on screen — same reasoning as `check_updates` running
/// after the fact rather than blocking. Concurrent per app for the same
/// reason `check_updates` is: a library of a dozen+ apps shouldn't
/// backfill one at a time.
#[tauri::command]
async fn backfill_icons() -> Result<Vec<BackfilledIcon>, String> {
    let entries = library::load().map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;

    let fetches = entries.into_iter().map(|entry| {
        let client = client.clone();
        async move {
            let path = flow::fetch_and_cache_worker_icon(&client, &entry.repo, &entry.slug).await?;
            library::set_icon(&entry.slug, path.clone()).ok()?;
            let bytes = tokio::fs::read(&path).await.ok()?;
            Some(BackfilledIcon {
                slug: entry.slug,
                icon_data_url: format!("data:image/svg+xml;base64,{}", STANDARD.encode(bytes)),
            })
        }
    });

    Ok(futures_util::future::join_all(fetches).await.into_iter().flatten().collect())
}

/// Re-runs an already-downloaded, already-checksum-verified app straight
/// from the local cache — deliberately does *not* require a fresh signed
/// link. The signature's job is authorizing what gets downloaded onto the
/// machine in the first place; once that's verified once, replaying it
/// locally doesn't need the site back in the loop.
#[tauri::command]
async fn relaunch(app: tauri::AppHandle, slug: String) -> Result<(), String> {
    let entry = library::find(&slug)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{slug} is not installed"))?;
    let cwd = install::sandbox_dir(&slug).map_err(|e| e.to_string())?;
    let title = entry.repo.rsplit('/').next().unwrap_or(&entry.repo).to_string();
    flow::launch_installed(&app, &slug, &title, &entry.path, &cwd, entry.is_gui)
        .await
        .map_err(|e| e.to_string())?;
    let _ = library::touch_last_launched(&slug);
    Ok(())
}

/// Removes an app from the gallery and deletes everything it downloaded —
/// there's no other way to manage what's installed, since installs
/// deliberately live in a hidden folder rather than somewhere Finder-visible.
#[tauri::command]
fn uninstall(slug: String) -> Result<(), String> {
    // Looked up before removal purely to have repo/commit on hand for the
    // best-effort event report below — uninstall itself doesn't need it.
    let entry = library::find(&slug).map_err(|e| e.to_string())?;

    install::remove_all(&slug).map_err(|e| e.to_string())?;
    library::remove(&slug).map_err(|e| e.to_string())?;

    if let (Some(entry), Some(acct)) = (entry, account::load().ok().flatten()) {
        tauri::async_runtime::spawn(async move {
            let client = reqwest::Client::new();
            let _ = orchestrator::report_install_event(
                &client,
                &acct.device_token,
                &entry.repo,
                Some(&entry.commit),
                "uninstalled",
            )
            .await;
        });
    }

    Ok(())
}

/// Untracks an app without touching anything it downloaded — distinct from
/// `uninstall`, which does both. This is for "stop showing this in my
/// library" (a stale/unwanted entry, a scratch build you don't want synced)
/// without losing the ability to just relaunch it later; the files are
/// still sitting in ~/.securexe/apps, just no longer in library.json or
/// reported as installed on this device.
///
/// Reports the same `"uninstalled"` action as `uninstall` above, to the
/// per-device install log — the backend's install-event vocabulary only
/// ever means "this device has it installed" or not, it has no separate
/// notion of "kept a local cache but stopped tracking it". This never
/// touches the account library (`GET /library` / `remove_from_account`
/// above) — that's a completely separate, explicit-add-only concept now;
/// removing a repo from your library and removing it from this device's
/// tracked installs are unrelated actions.
#[tauri::command]
fn remove_from_library(slug: String) -> Result<(), String> {
    let entry = library::find(&slug).map_err(|e| e.to_string())?;
    library::remove(&slug).map_err(|e| e.to_string())?;

    if let (Some(entry), Some(acct)) = (entry, account::load().ok().flatten()) {
        tauri::async_runtime::spawn(async move {
            let client = reqwest::Client::new();
            let _ = orchestrator::report_install_event(
                &client,
                &acct.device_token,
                &entry.repo,
                Some(&entry.commit),
                "uninstalled",
            )
            .await;
        });
    }

    Ok(())
}

/// One installed app with a newer build available than what's currently
/// on disk (`library::LibraryEntry.commit`). The frontend never sees a
/// commit hash for its own sake — just enough to know which tiles get the
/// update badge.
#[derive(Serialize)]
struct UpdateStatus {
    slug: String,
}

/// Asks the worker which installed apps are stale via `GET /updates` — one
/// call for the whole library instead of fetching every app's manifest and
/// comparing commits client-side (see `orchestrator::fetch_updates`). Needs
/// a linked device; an unlinked launcher has nothing to check against and
/// just reports nothing updatable, same as any other failure here.
#[tauri::command]
async fn check_updates() -> Result<Vec<UpdateStatus>, String> {
    let Some(acct) = account::load().map_err(|e| e.to_string())? else {
        return Ok(Vec::new());
    };
    let entries = library::load().map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;

    let stale_repos: HashSet<String> = orchestrator::fetch_updates(&client, &acct.device_token)
        .await
        .map(|updates| updates.into_iter().map(|u| u.repo).collect())
        .unwrap_or_default();

    Ok(entries
        .into_iter()
        .filter(|e| stale_repos.contains(&e.repo))
        .map(|e| UpdateStatus { slug: e.slug })
        .collect())
}

/// Updates one app to whatever the orchestrator currently reports as the
/// latest build for its repo, then launches it — reusing the exact same
/// fetch/verify/install/launch path (and the same `launcher-status`
/// progress events) as opening a fresh `securexe://run` link. See
/// `flow::install_and_launch` for why this doesn't need a signature here.
#[tauri::command]
async fn update_slug(app: tauri::AppHandle, slug: String) -> Result<(), String> {
    let entry = library::find(&slug)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{slug} is not installed"))?;
    flow::install_and_launch(&app, entry.repo, slug, None, true)
        .await
        .map_err(|e| e.to_string())
}

/// Updates every installed app that's currently behind, without launching
/// any of them — unlike `update_slug` above (reachable only by right-
/// clicking one specific tile, where relaunching that one app afterward
/// makes sense), this is a "bring the whole library up to date" action, and
/// popping open a window for every app that happened to be stale would be
/// unwanted. Re-checks staleness itself rather than trusting the frontend's
/// `updatable` set, since that's just a cache of whatever `check_updates`
/// last reported and may be out of date by the time the user clicks
/// through. Updates sequentially (not concurrently) so the shared
/// `launcher-status` banner shows one coherent progression instead of
/// several apps' steps interleaved. Same "one bad repo shouldn't sink the
/// rest" philosophy as `check_updates`: a failure just gets logged and its
/// slug added to the returned list, everything else still gets updated.
#[tauri::command]
async fn update_all(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let Some(acct) = account::load().map_err(|e| e.to_string())? else {
        return Ok(Vec::new());
    };
    let entries = library::load().map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;

    let stale_repos: HashSet<String> = orchestrator::fetch_updates(&client, &acct.device_token)
        .await
        .map(|updates| updates.into_iter().map(|u| u.repo).collect())
        .unwrap_or_default();
    let stale: Vec<_> = entries.into_iter().filter(|e| stale_repos.contains(&e.repo)).collect();

    let mut failed = Vec::new();
    for entry in stale {
        if let Err(e) = flow::install_and_launch(&app, entry.repo, entry.slug.clone(), None, false).await {
            eprintln!("[update_all] failed to update {}: {e}", entry.slug);
            failed.push(entry.slug);
        }
    }
    Ok(failed)
}

/// Sanitized account view for the webview — never the raw device_token,
/// same discipline as GalleryEntry never exposing raw filesystem paths.
#[derive(Serialize)]
struct AccountInfo {
    github_username: String,
    linked_at: u64,
}

#[tauri::command]
fn get_account() -> Result<Option<AccountInfo>, String> {
    account::load()
        .map(|opt| {
            opt.map(|a| AccountInfo {
                github_username: a.github_username,
                linked_at: a.linked_at,
            })
        })
        .map_err(|e| e.to_string())
}

/// Tells the worker this device is disconnecting before forgetting the
/// device token locally — the token is the only credential that call can
/// authenticate with, so it has to happen before account::clear(), not
/// after. Best-effort: local state clears regardless of whether the worker
/// call succeeds, since the user explicitly asked to unlink and a network
/// hiccup shouldn't trap them in a "linked" state they can't get out of.
#[tauri::command]
async fn unlink() -> Result<(), String> {
    if let Some(acct) = account::load().ok().flatten() {
        let client = reqwest::Client::new();
        let _ = orchestrator::unlink_device(&client, &acct.device_token).await;
    }
    account::clear().map_err(|e| e.to_string())
}

/// Checks whether a newer release of securexe-launcher itself is
/// available — separate from `check_updates` above, which is about
/// installed *applets*. `app.package_info().version` is stamped from the
/// release tag at build time (see release.yml); a locally-built dev
/// binary reports whatever Cargo.toml says (0.1.0, never bumped in the
/// repo itself), which self_update::check treats as "not ahead of any
/// real release" rather than erroring.
#[tauri::command]
async fn check_launcher_update(app: tauri::AppHandle) -> Result<Option<self_update::LauncherUpdate>, String> {
    let current = app.package_info().version.to_string();
    self_update::check(&current).await.map_err(|e| e.to_string())
}

/// Opens `url` in the system's default browser — used as the fallback
/// for `install_launcher_update` on platforms without a scripted
/// installer yet. Thin wrapper so the frontend stays consistent with how
/// every other action here goes through a named command, rather than
/// reaching for the opener plugin's raw `plugin:opener|open_url` IPC name
/// directly.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// Installs the update the update banner is currently showing. On macOS
/// this runs the real thing (see self_update::update_via_terminal): a
/// visible Terminal script downloads, installs, de-quarantines, and
/// relaunches, with no manual drag-and-drop. Windows/Linux don't have
/// that scripted installer yet, so they fall back to opening the release
/// page — nothing here claims to automate a platform it doesn't actually
/// automate.
#[tauri::command]
fn install_launcher_update(download_url: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let _ = &download_url; // macOS derives the URL itself — see self_update::update_via_terminal
        self_update::update_via_terminal().map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        tauri_plugin_opener::open_url(download_url, None::<&str>).map_err(|e| e.to_string())
    }
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
    // Branch on the action (`run` vs `link`) before the dedup check below
    // eats a plain string, since both share the same `securexe://<action>`
    // shape and this is the one place that decides which flow handles a
    // given incoming URL. Any other/missing action still falls through to
    // handle_run_url, which already produces the right "unsupported
    // action" error via repo::parse_run_url — unchanged from before.
    let action = url.host_str().unwrap_or_default().to_string();
    let url = url.to_string();
    if already_dispatched(&url) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        match action.as_str() {
            "link" => flow::handle_link_url(app, url).await,
            _ => flow::handle_run_url(app, url).await,
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .invoke_handler(tauri::generate_handler![
            list_library,
            list_account_library,
            remove_from_account,
            browse_catalog,
            install_from_catalog,
            backfill_icons,
            relaunch,
            uninstall,
            remove_from_library,
            check_updates,
            update_slug,
            update_all,
            get_account,
            unlink,
            check_launcher_update,
            open_url,
            install_launcher_update
        ])
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
