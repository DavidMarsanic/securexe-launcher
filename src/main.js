const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const galleryEl = document.querySelector("#gallery");
const emptyStateEl = document.querySelector("#empty-state");
const bannerEl = document.querySelector("#status-banner");
const bannerRepoEl = document.querySelector("#status-repo");
const bannerTextEl = document.querySelector("#status-text");

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
    label.textContent = entry.repo.split("/").pop();

    tile.append(img, label);
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

listen("launcher-status", (event) => {
  const payload = event.payload;
  bannerEl.classList.remove("hidden");

  if (payload.step === "error") {
    bannerRepoEl.textContent = "";
    bannerTextEl.textContent = `Error: ${payload.message}`;
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
