const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const galleryEl = document.querySelector("#gallery");
const emptyStateEl = document.querySelector("#empty-state");
const bannerEl = document.querySelector("#status-banner");
const bannerRepoEl = document.querySelector("#status-repo");
const bannerTextEl = document.querySelector("#status-text");
const accountBarEl = document.querySelector("#account-bar");
const accountLabelEl = document.querySelector("#account-label");
const unlinkBtnEl = document.querySelector("#unlink-btn");

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

// Tauri's WKWebView doesn't reliably implement native dialogs like
// window.confirm() unless the host app wires that up itself, which this
// doesn't — so a native confirm() here can silently no-op instead of
// prompting. Click-twice avoids depending on that entirely: the first
// click arms it, the second (within 3s) commits. Shared by the uninstall
// and remove-from-library buttons below, which are otherwise identical
// interactions with different destinations.
function makeConfirmButton({ className, symbol, idleTitle, confirmTitle, onConfirm }) {
  const btn = document.createElement("span");
  btn.className = className;
  btn.textContent = symbol;
  btn.title = idleTitle;
  let confirmTimer = null;

  btn.addEventListener("click", async (e) => {
    e.stopPropagation();

    if (!btn.classList.contains("confirming")) {
      btn.classList.add("confirming");
      btn.textContent = "✓";
      btn.title = confirmTitle;
      confirmTimer = setTimeout(() => {
        btn.classList.remove("confirming");
        btn.textContent = symbol;
        btn.title = idleTitle;
      }, 3000);
      return;
    }

    clearTimeout(confirmTimer);
    await onConfirm();
  });

  return btn;
}

function renderGallery(entries) {
  galleryEl.innerHTML = "";
  emptyStateEl.classList.toggle("hidden", entries.length > 0);

  const sorted = [...entries].sort((a, b) => {
    const aTime = a.last_launched_at ?? a.installed_at;
    const bTime = b.last_launched_at ?? b.installed_at;
    return bTime - aTime;
  });

  for (const entry of sorted) {
    const tile = document.createElement("button");
    tile.className = "app-tile";
    tile.title = entry.repo;

    const img = document.createElement("img");
    img.src = entry.icon_data_url ?? DEFAULT_ICON;
    img.alt = "";

    const label = document.createElement("span");
    label.className = "app-tile-label";
    label.textContent = entry.repo.split("/").pop();

    // Destructive: deletes the actual download too.
    const uninstallBtn = makeConfirmButton({
      className: "app-tile-remove",
      symbol: "×",
      idleTitle: `Uninstall ${entry.repo}`,
      confirmTitle: `Click again to uninstall ${entry.repo}`,
      onConfirm: async () => {
        try {
          await invoke("uninstall", { slug: entry.slug });
          refreshGallery();
        } catch (err) {
          console.error("uninstall failed", err);
        }
      },
    });

    // Non-destructive: drops it from the library (local and synced) but
    // leaves the downloaded files alone — relaunching later re-adds it.
    const untrackBtn = makeConfirmButton({
      className: "app-tile-untrack",
      symbol: "−",
      idleTitle: `Remove ${entry.repo} from library (keeps the download)`,
      confirmTitle: `Click again to remove ${entry.repo} from your library`,
      onConfirm: async () => {
        try {
          await invoke("remove_from_library", { slug: entry.slug });
          refreshGallery();
        } catch (err) {
          console.error("remove from library failed", err);
        }
      },
    });

    tile.append(img, label, uninstallBtn, untrackBtn);
    tile.addEventListener("click", async () => {
      // A double-click fires two separate DOM click events, not one. A
      // local relaunch resolves in single-digit milliseconds, well before
      // a real double-click's second click event even arrives (~150-300ms
      // later) — so disabling only for the invoke's own duration doesn't
      // catch it. Hold the tile disabled for a fixed cooldown instead, so
      // a single click and a double-click both collapse into exactly one
      // launch regardless of how fast relaunch itself finishes.
      if (tile.disabled) return;
      tile.disabled = true;
      const launch = invoke("relaunch", { slug: entry.slug }).catch((e) => {
        console.error("relaunch failed", e);
      });
      await Promise.all([launch, new Promise((resolve) => setTimeout(resolve, 600))]);
      tile.disabled = false;
    });
    galleryEl.appendChild(tile);
  }
}

async function refreshGallery() {
  try {
    const entries = await invoke("list_library");
    renderGallery(entries);
  } catch (e) {
    console.error("failed to load library", e);
  }
}

// Linking only ever happens by receiving a `securexe://link` deep link from
// the website — there's nothing to click in here to initiate it, only to
// undo it, so this bar is purely a status readout plus an unlink escape
// hatch.
function renderAccount(account) {
  accountBarEl.classList.toggle("hidden", !account);
  if (account) {
    accountLabelEl.textContent = `Linked as ${account.github_username}`;
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

refreshGallery();
refreshAccount();
