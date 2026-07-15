const { listen } = window.__TAURI__.event;

const repoNameEl = document.querySelector("#repo-name");
const statusEl = document.querySelector("#status");

const STEP_LABEL = {
  resolving: "Resolving…",
  downloading: "Downloading…",
  verifying: "Verifying…",
  launching: "Launching…",
  done: "Launched.",
};

listen("launcher-status", (event) => {
  const payload = event.payload;

  if (payload.step === "error") {
    statusEl.textContent = `Error: ${payload.message}`;
    return;
  }

  if (payload.repo) {
    repoNameEl.textContent = payload.repo;
  }
  statusEl.textContent = STEP_LABEL[payload.step] ?? payload.step;
});
