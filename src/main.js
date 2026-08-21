const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const myAppsViewEl = document.querySelector("#my-apps-view");
const installedGalleryEl = document.querySelector("#installed-gallery");
const installedEmptyStateEl = document.querySelector("#installed-empty-state");
const libraryGalleryEl = document.querySelector("#library-gallery");
const libraryNoteEl = document.querySelector("#library-note");
const librarySignInRowEl = document.querySelector("#library-signin-row");
const librarySignInBtnEl = document.querySelector("#library-signin-btn");
const browseViewEl = document.querySelector("#browse-view");
const browseGalleryEl = document.querySelector("#browse-gallery");
const browseEmptyStateEl = document.querySelector("#browse-empty-state");
const tabBtnEls = document.querySelectorAll(".tab-btn");
const searchInputEl = document.querySelector("#search-input");
const bannerEl = document.querySelector("#status-banner");
const bannerRepoEl = document.querySelector("#status-repo");
const bannerTextEl = document.querySelector("#status-text");
const accountBarEl = document.querySelector("#account-bar");
const accountLabelEl = document.querySelector("#account-label");
const unlinkBtnEl = document.querySelector("#unlink-btn");
const launcherUpdateBannerEl = document.querySelector("#launcher-update-banner");
const launcherUpdateTextEl = document.querySelector("#launcher-update-text");
const launcherUpdateDownloadBtnEl = document.querySelector("#launcher-update-download-btn");
const updateAllBannerEl = document.querySelector("#update-all-banner");
const updateAllTextEl = document.querySelector("#update-all-text");
const updateAllBtnEl = document.querySelector("#update-all-btn");

const STEP_LABEL = {
  resolving: "Resolving…",
  downloading: "Downloading…",
  verifying: "Verifying…",
  launching: "Launching…",
  done: "Launched.",
};

// Generic placeholder for CLI tools, which have no bundled icon of their
// own — GUI apps get their real extracted icon instead (see icon_data_url).
const DEFAULT_ICON =
  "data:image/svg+xml;utf8," +
  encodeURIComponent(`
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none">
      <rect x="1" y="1" width="22" height="22" rx="5" fill="#8a8aa3"/>
      <path d="M6 9l3.5 3.5L6 16" stroke="#fff" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>
      <line x1="12" y1="16" x2="18" y2="16" stroke="#fff" stroke-width="1.6" stroke-linecap="round"/>
    </svg>
  `);

// ---- tabs + search --------------------------------------------------------

let activeTab = "my-apps"; // "my-apps" | "browse"
let searchQuery = "";
let searchDebounceTimer = null;

// Cached results so switching tabs or re-filtering doesn't require a fresh
// fetch every time — only the thing that actually changed (a new search
// query on Browse, a fresh install/uninstall on Installed) re-fetches.
// My Apps has two independent sources: `installedEntries` (this device's
// disk state — always available) and `libraryEntries` (the account's
// library minus whatever's already installed here — currently blocked,
// see refreshAccountLibrary below).
let installedEntries = [];
let libraryEntries = [];
let libraryError = null;
let browseEntries = [];

// Set by renderAccount below — `renderLibrary` reads this to tell "you're
// signed in with nothing else to show" apart from "you're not signed in at
// all," which `list_account_library` can't distinguish on its own (it
// returns an empty list either way, see lib.rs).
let linkedAccount = null;

function matchesQuery(repo) {
  if (!searchQuery) return true;
  return repo.toLowerCase().includes(searchQuery);
}

tabBtnEls.forEach((btn) => {
  btn.addEventListener("click", () => {
    if (btn.dataset.tab === activeTab) return;
    activeTab = btn.dataset.tab;
    tabBtnEls.forEach((b) => {
      b.classList.toggle("active", b === btn);
      b.setAttribute("aria-selected", b === btn ? "true" : "false");
    });
    renderActiveTab();
  });
});

searchInputEl.addEventListener("input", () => {
  clearTimeout(searchDebounceTimer);
  searchDebounceTimer = setTimeout(() => {
    searchQuery = searchInputEl.value.trim().toLowerCase();
    renderActiveTab();
  }, 200);
});

// Re-renders (or, for Browse, re-fetches) whatever tab is currently active
// — the single place every tab-switch, search keystroke, and post-mutation
// refresh (install/uninstall/relaunch) funnels through, so each of those
// only has to worry about updating its own cached list, not the DOM.
function renderActiveTab() {
  myAppsViewEl.classList.toggle("hidden", activeTab !== "my-apps");
  browseViewEl.classList.toggle("hidden", activeTab !== "browse");

  if (activeTab === "my-apps") {
    renderInstalled(installedEntries.filter((e) => matchesQuery(e.repo)));
    renderLibrary(libraryEntries.filter((e) => matchesQuery(e.repo)));
    return;
  }

  // "browse" — re-fetch on every query change since the worker does the
  // matching server-side (see orchestrator::search_catalog); this isn't a
  // client-side filter over a locally cached full catalog.
  fetchAndRenderBrowse();
}

async function fetchAndRenderBrowse() {
  try {
    browseEntries = await invoke("browse_catalog", { query: searchQuery || null });
  } catch (e) {
    console.error("browse_catalog failed", e);
    browseEntries = [];
  }
  if (activeTab === "browse") renderBrowse(browseEntries);
}

// The account-wide half of My Apps — everything in this account's library
// that isn't already sitting in `installedEntries`. Fetched separately from
// `refreshGallery`'s local `list_library` call (not blocking it) since this
// one talks to the worker and can fail — as of this writing it always does,
// because `GET /library` only accepts a session token today and the
// launcher only ever holds a device token (see `list_account_library` in
// lib.rs). `libraryError` carries that forward so `renderLibrary` can say
// so plainly instead of quietly rendering an empty section that looks like
// "you have nothing else," which isn't what's actually happening.
async function refreshAccountLibrary() {
  try {
    libraryEntries = await invoke("list_account_library");
    libraryError = null;
  } catch (e) {
    console.error("list_account_library failed", e);
    libraryEntries = [];
    libraryError = String(e);
  }
  if (activeTab === "my-apps") {
    renderLibrary(libraryEntries.filter((e) => matchesQuery(e.repo)));
  }
}

// Slugs check_updates last reported as having a newer build available.
// Populated asynchronously after the gallery itself renders — updates
// shouldn't block getting tiles on screen — and re-checked whenever the
// window regains focus, since this app is typically opened briefly rather
// than left running.
let updatable = new Set();

function applyUpdateBadges(container) {
  for (const tile of container.querySelectorAll(".app-tile")) {
    tile.classList.toggle("has-update", updatable.has(tile.dataset.slug));
  }
}

// Only worth surfacing as its own prompt once there's more than one app to
// update — a single stale app is already covered by its tile's pulsing
// badge and the "Update available" item in its right-click menu.
function refreshUpdateAllBanner() {
  if (updateAllBtnEl.disabled) return; // an update-all run is in flight
  if (updatable.size > 1) {
    updateAllTextEl.textContent = `${updatable.size} updates available.`;
    updateAllBannerEl.classList.remove("hidden");
  } else {
    updateAllBannerEl.classList.add("hidden");
  }
}

async function checkUpdates() {
  try {
    const results = await invoke("check_updates");
    updatable = new Set(results.map((r) => r.slug));
    applyUpdateBadges();
    refreshUpdateAllBanner();
  } catch (e) {
    console.error("check_updates failed", e);
  }
}

updateAllBtnEl.addEventListener("click", async () => {
  updateAllBtnEl.disabled = true;
  updateAllTextEl.textContent = "Updating all…";

  let failed = [];
  try {
    failed = await invoke("update_all");
  } catch (e) {
    console.error("update_all failed", e);
    updateAllBtnEl.disabled = false;
    updateAllTextEl.textContent = `Update all failed: ${e}`;
    return;
  }

  updateAllBtnEl.disabled = false;
  await checkUpdates(); // refreshes badges + this banner from the new update set
  if (failed.length > 0) {
    updateAllTextEl.textContent = `Updated, but ${failed.length} failed: ${failed.join(", ")}`;
    updateAllBannerEl.classList.remove("hidden");
  }
});

let pendingLauncherDownloadUrl = null;

// Distinct from checkUpdates above (per-app tile badges) — this is about
// securexe-launcher itself. Same load-and-on-focus cadence as the app
// update check, since this window is typically opened briefly rather
// than left running, so focus is the main signal that time has passed.
async function checkLauncherUpdate() {
  try {
    const update = await invoke("check_launcher_update");
    if (!update) {
      launcherUpdateBannerEl.classList.add("hidden");
      pendingLauncherDownloadUrl = null;
      return;
    }
    pendingLauncherDownloadUrl = update.download_url;
    launcherUpdateTextEl.textContent = `A new version of Brightencode Launcher is available (v${update.version}).`;
    launcherUpdateBannerEl.classList.remove("hidden");
  } catch (e) {
    console.error("check_launcher_update failed", e);
  }
}

launcherUpdateDownloadBtnEl.addEventListener("click", () => {
  if (pendingLauncherDownloadUrl) {
    launcherUpdateDownloadBtnEl.disabled = true;
    launcherUpdateTextEl.textContent = "Updating — check the Terminal window that just opened…";
    invoke("install_launcher_update", { downloadUrl: pendingLauncherDownloadUrl }).catch((e) => {
      console.error("install_launcher_update failed", e);
      launcherUpdateDownloadBtnEl.disabled = false;
      launcherUpdateTextEl.textContent = `Update failed: ${e}`;
    });
  }
});

// ---- right-click context menu -----------------------------------------

let openMenu = null;

function closeMenu() {
  if (!openMenu) return;
  openMenu.remove();
  openMenu = null;
  document.removeEventListener("pointerdown", handleOutsideClick, true);
  document.removeEventListener("keydown", handleEscape, true);
}

function handleOutsideClick(e) {
  if (openMenu && !openMenu.contains(e.target)) closeMenu();
}

function handleEscape(e) {
  if (e.key === "Escape") closeMenu();
}

// Tauri's WKWebView doesn't reliably implement native dialogs like
// window.confirm() unless the host app wires that up itself, which this
// doesn't — so a real confirm() here can silently no-op instead of
// prompting. Click-twice avoids depending on that entirely: the first
// click arms it, the second (within 3s) commits.
function makeMenuItem({ className, label, confirmLabel, onActivate }) {
  const item = document.createElement("button");
  item.className = className;
  item.type = "button";
  item.textContent = label;

  if (!confirmLabel) {
    item.addEventListener("click", (e) => {
      e.stopPropagation();
      closeMenu();
      onActivate();
    });
    return item;
  }

  let confirmTimer = null;
  item.addEventListener("click", (e) => {
    e.stopPropagation();
    if (!item.classList.contains("confirming")) {
      item.classList.add("confirming");
      item.textContent = confirmLabel;
      confirmTimer = setTimeout(() => {
        item.classList.remove("confirming");
        item.textContent = label;
      }, 3000);
      return;
    }
    clearTimeout(confirmTimer);
    closeMenu();
    onActivate();
  });
  return item;
}

// `kind` picks which actions apply — they key off different identifiers
// and hit different commands:
// - "installed": a tile actually on this device (Installed section, or a
//   Browse tile that happens to be installed). Uninstall/Remove both key
//   off `slug` and touch local state + report to the backend.
// - "library": a My Apps library-section tile — in the account's library,
//   but not on this device, so there's no local `slug`/library.json entry
//   to act on. Remove is a pure backend call keyed off `repo` instead (see
//   `remove_from_account` in lib.rs).
function showContextMenu(x, y, entry, kind) {
  closeMenu();

  const menu = document.createElement("div");
  menu.className = "context-menu";

  if (kind === "library") {
    menu.appendChild(
      makeMenuItem({
        className: "context-menu-item",
        label: "Remove from library",
        confirmLabel: "Click again to remove",
        onActivate: async () => {
          try {
            await invoke("remove_from_account", { repo: entry.repo });
            libraryEntries = libraryEntries.filter((e) => e.repo !== entry.repo);
            renderActiveTab();
          } catch (err) {
            console.error("remove from account failed", err);
          }
        },
      })
    );
    document.body.appendChild(menu);
    positionAndOpenMenu(menu, x, y);
    return;
  }

  // Most prominent thing in the menu, and the only reason this menu might
  // open with something already "armed" to draw the eye — everything else
  // here is equally available at any time, but an available update is the
  // one thing worth surfacing proactively.
  if (updatable.has(entry.slug)) {
    const updateItem = makeMenuItem({
      className: "context-menu-item context-menu-update",
      label: "● Update available",
      onActivate: async () => {
        try {
          await invoke("update_slug", { slug: entry.slug });
          // install_and_launch also emits a "done" launcher-status event,
          // which already triggers a full refreshGallery() — this just
          // clears the badge immediately rather than waiting on that
          // round-trip.
          updatable.delete(entry.slug);
          applyUpdateBadges(installedGalleryEl);
          applyUpdateBadges(browseGalleryEl);
        } catch (err) {
          console.error("update failed", err);
        }
      },
    });
    menu.appendChild(updateItem);
    menu.appendChild(Object.assign(document.createElement("div"), { className: "context-menu-separator" }));
  }

  menu.appendChild(
    makeMenuItem({
      className: "context-menu-item context-menu-destructive",
      label: "Uninstall",
      confirmLabel: "Click again to uninstall",
      onActivate: async () => {
        try {
          await invoke("uninstall", { slug: entry.slug });
          refreshGallery();
        } catch (err) {
          console.error("uninstall failed", err);
        }
      },
    })
  );

  menu.appendChild(
    makeMenuItem({
      className: "context-menu-item",
      label: "Remove from library",
      confirmLabel: "Click again to remove",
      onActivate: async () => {
        try {
          await invoke("remove_from_library", { slug: entry.slug });
          refreshGallery();
        } catch (err) {
          console.error("remove from library failed", err);
        }
      },
    })
  );

  document.body.appendChild(menu);
  positionAndOpenMenu(menu, x, y);
}

function positionAndOpenMenu(menu, x, y) {
  const rect = menu.getBoundingClientRect();
  const left = Math.max(4, Math.min(x, window.innerWidth - rect.width - 4));
  const top = Math.max(4, Math.min(y, window.innerHeight - rect.height - 4));
  menu.style.left = `${left}px`;
  menu.style.top = `${top}px`;

  openMenu = menu;
  // Deferred so the contextmenu event's own click/pointerdown doesn't
  // immediately trigger the outside-click handler that just got attached.
  setTimeout(() => {
    document.addEventListener("pointerdown", handleOutsideClick, true);
    document.addEventListener("keydown", handleEscape, true);
  }, 0);
}

// ---- gallery ------------------------------------------------------------

// Builds one tile shared by all three sections (Installed, My Apps'
// library section, Browse) — they only differ in where the icon comes
// from, what a click does, and which optional decorations apply, not in
// the tile's basic shape.
//
// `action` returns the invoke() promise for the click; a double-click
// fires two separate DOM click events, not one, and a local relaunch can
// resolve in single-digit milliseconds — well before a real double-click's
// second click event even arrives (~150-300ms later) — so disabling only
// for the invoke's own duration doesn't catch it. Holding the tile disabled
// for a fixed cooldown instead collapses a single click and a double-click
// into exactly one action regardless of how fast it finishes.
function makeAppTile(entry, { iconSrc, action, actionLabel, showInstallBadge, contextMenuKind, disabled, disabledTitle }) {
  const tile = document.createElement("button");
  tile.className = "app-tile";
  tile.title = disabled ? disabledTitle : entry.repo;
  tile.dataset.slug = entry.slug;
  tile.disabled = !!disabled;

  const img = document.createElement("img");
  img.src = iconSrc ?? DEFAULT_ICON;
  img.alt = "";

  const label = document.createElement("span");
  label.className = "app-tile-label";
  label.textContent = entry.repo.split("/").pop();

  const updateBadge = document.createElement("span");
  updateBadge.className = "app-tile-update-badge";
  updateBadge.title = "Update available";

  tile.append(img, updateBadge, label);

  if (showInstallBadge) {
    const badge = document.createElement("span");
    badge.className = "app-tile-install-badge";
    badge.textContent = "Install";
    tile.appendChild(badge);
  }

  if (contextMenuKind) {
    tile.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      showContextMenu(e.clientX, e.clientY, entry, contextMenuKind);
    });
  }

  tile.addEventListener("click", async () => {
    if (tile.disabled) return;
    tile.disabled = true;
    const settled = action().catch((e) => console.error(`${actionLabel} failed`, e));
    await Promise.all([settled, new Promise((resolve) => setTimeout(resolve, 600))]);
    tile.disabled = !!disabled;
  });

  return tile;
}

function renderInstalled(entries) {
  installedGalleryEl.innerHTML = "";
  installedEmptyStateEl.classList.toggle("hidden", entries.length > 0);
  installedEmptyStateEl.textContent = searchQuery
    ? `No installed apps match "${searchQuery}".`
    : "No apps yet — click Download on a project on the site to get started.";

  const sorted = [...entries].sort((a, b) => {
    const aTime = a.last_launched_at ?? a.installed_at;
    const bTime = b.last_launched_at ?? b.installed_at;
    return bTime - aTime;
  });

  for (const entry of sorted) {
    installedGalleryEl.appendChild(
      makeAppTile(entry, {
        iconSrc: entry.icon_data_url,
        action: () => invoke("relaunch", { slug: entry.slug }),
        actionLabel: "relaunch",
        contextMenuKind: "installed",
      })
    );
  }

  applyUpdateBadges(installedGalleryEl);
}

// The rest of this account's library — repos added to it (on this device
// or another) that aren't installed here. Not the same list as Installed:
// this is what `list_account_library` returns after excluding whatever's
// already local (see lib.rs). Every tile here is a click-to-install, same
// as a not-yet-installed Browse tile.
function renderLibrary(entries) {
  libraryGalleryEl.innerHTML = "";

  // Not signed in: `list_account_library` returns an empty list rather than
  // an error in this case (see lib.rs), so without this check every
  // unlinked user would see "Nothing else — everything in your library is
  // already installed here," which is a lie — there's no library to check
  // against at all. Show the sign-in prompt instead and skip the rest of
  // the note logic entirely.
  if (!linkedAccount) {
    libraryNoteEl.classList.add("hidden");
    librarySignInRowEl.classList.remove("hidden");
    return;
  }
  librarySignInRowEl.classList.add("hidden");

  if (libraryError) {
    libraryNoteEl.textContent = "Couldn't load your account library right now.";
    libraryNoteEl.classList.remove("hidden");
  } else if (entries.length === 0) {
    libraryNoteEl.textContent = searchQuery
      ? `No other library apps match "${searchQuery}".`
      : "Nothing else — everything in your library is already installed here.";
    libraryNoteEl.classList.remove("hidden");
  } else {
    libraryNoteEl.classList.add("hidden");
  }

  for (const entry of entries) {
    libraryGalleryEl.appendChild(
      makeAppTile(entry, {
        iconSrc: entry.icon_url,
        action: () => invoke("install_from_catalog", { repo: entry.repo }),
        actionLabel: "install",
        showInstallBadge: true,
        contextMenuKind: "library",
      })
    );
  }
}

// Browse-tab tiles come from the public catalog (`browse_catalog`, backed
// by the worker's `/search`), not the local library — most aren't
// installed yet, so a tile's primary action is Install rather than a bare
// relaunch, and `entry.icon_url` is a live remote URL rather than the
// `icon_data_url` the installed gallery caches locally (see CatalogEntry
// in lib.rs for why: no reason to fetch/cache an icon for every catalog
// entry when the worker already serves it directly).
function renderBrowse(entries) {
  browseGalleryEl.innerHTML = "";
  browseEmptyStateEl.classList.toggle("hidden", entries.length > 0);
  browseEmptyStateEl.textContent = searchQuery
    ? `No catalog results for "${searchQuery}".`
    : "The catalog is empty.";

  for (const entry of entries) {
    browseGalleryEl.appendChild(
      makeAppTile(entry, {
        iconSrc: entry.icon_url,
        action: () =>
          entry.installed ? invoke("relaunch", { slug: entry.slug }) : invoke("install_from_catalog", { repo: entry.repo }),
        actionLabel: entry.installed ? "relaunch" : "install",
        showInstallBadge: !entry.installed && entry.available,
        // Uninstall/Remove only make sense for a tile that's actually on
        // this device — same "installed" context menu the Installed
        // section uses, since CatalogEntry carries the same slug/repo
        // shape showContextMenu already reads.
        contextMenuKind: entry.installed ? "installed" : null,
        disabled: !entry.available,
        disabledTitle: `${entry.repo} — not built for this platform yet`,
      })
    );
  }

  applyUpdateBadges(browseGalleryEl);
}

// Fetches the worker's generated icon (same one securexe-web's catalog
// shows via iconUrl) for any tile that's still on the generic placeholder
// — apps installed before this existed, or whose install-time fetch
// failed. Runs after the gallery's initial paint so a slow/offline
// network never delays getting tiles on screen, same reasoning as
// checkUpdates below.
async function backfillIcons() {
  try {
    const filled = await invoke("backfill_icons");
    for (const { slug, icon_data_url } of filled) {
      // Patch the cached entry too, not just the DOM — the Installed tab
      // isn't necessarily what's on screen right now (Browse/My Apps might
      // be), and a later tab switch re-renders from `installedEntries`
      // rather than re-fetching, so a DOM-only patch would get lost the
      // moment the user switches away and back.
      const entry = installedEntries.find((e) => e.slug === slug);
      if (entry) entry.icon_data_url = icon_data_url;

      const img = installedGalleryEl.querySelector(`.app-tile[data-slug="${CSS.escape(slug)}"] img`);
      if (img) img.src = icon_data_url;
    }
  } catch (e) {
    console.error("backfill_icons failed", e);
  }
}

async function refreshGallery() {
  try {
    installedEntries = await invoke("list_library");
  } catch (e) {
    console.error("failed to load library", e);
    installedEntries = [];
  }
  renderActiveTab();
  checkUpdates();
  backfillIcons();
  refreshAccountLibrary();
}

// Linking only ever completes by receiving a `securexe://link` deep link
// from the website — there's no way to mint that token in here, since it
// has to be signed server-side against a real session (see signing.ts in
// securexe-web). This bar is a status readout plus an unlink escape hatch;
// the actual sign-in entry point lives in the library section's sign-in
// button below, which just opens the browser to where that token gets minted.
function renderAccount(account) {
  linkedAccount = account;
  accountBarEl.classList.toggle("hidden", !account);
  if (account) {
    accountLabelEl.textContent = `Linked as ${account.github_username}`;
  }
  // refreshGallery and refreshAccount race at startup (see the bottom of
  // this file), so whichever of them lands last needs to re-render the
  // library section against the other's already-settled state — otherwise
  // a linked account that resolves after the (empty, pre-link) library
  // fetch would get stuck showing the sign-in prompt.
  if (activeTab === "my-apps") {
    renderLibrary(libraryEntries.filter((e) => matchesQuery(e.repo)));
  }
}

async function refreshAccount() {
  try {
    const account = await invoke("get_account");
    renderAccount(account);
  } catch (e) {
    console.error("failed to load account", e);
  }
}

unlinkBtnEl.addEventListener("click", async () => {
  try {
    await invoke("unlink");
    refreshAccount();
  } catch (e) {
    console.error("unlink failed", e);
  }
});

// Linking itself can't be initiated in here (see the note above
// renderAccount) — this just hands off to the website's /launcherlink
// route, which signs a `securexe://link` token from the visitor's session
// and fires it straight back at this app. /launcherlink covers both cases
// in one click: already signed in there gets the token immediately, signed
// out goes through GitHub first and bounces right back to finish the same
// handoff.
librarySignInBtnEl.addEventListener("click", () => {
  invoke("open_url", { url: "https://www.brightencode.com/launcherlink" }).catch((e) => {
    console.error("open_url failed", e);
  });
});

listen("launcher-status", (event) => {
  const payload = event.payload;
  bannerEl.classList.remove("hidden");

  if (payload.step === "error") {
    bannerRepoEl.textContent = "";
    bannerTextEl.textContent = `Error: ${payload.message}`;
    return;
  }

  if (payload.step === "linked") {
    bannerRepoEl.textContent = "";
    bannerTextEl.textContent = `Linked as ${payload.user}`;
    refreshAccount();
    refreshAccountLibrary();
    setTimeout(() => bannerEl.classList.add("hidden"), 2000);
    return;
  }

  if (payload.repo) {
    bannerRepoEl.textContent = payload.repo;
  }
  bannerTextEl.textContent = STEP_LABEL[payload.step] ?? payload.step;

  if (payload.step === "done") {
    refreshGallery();
    setTimeout(() => bannerEl.classList.add("hidden"), 2000);
  }
});

window.addEventListener("focus", checkUpdates);
window.addEventListener("focus", checkLauncherUpdate);

refreshGallery();
refreshAccount();
checkLauncherUpdate();
