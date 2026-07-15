# securexe-launcher

A small, cross-platform desktop helper — the "install + launch" half of the Securexe platform. It does not browse, build, or manage a catalog; that lives on the website. Its job is narrow: register the `securexe://` URL protocol, and when invoked, download the right binary from the [securexe-worker](.) orchestrator and run it.

## How it works (v1)

1. User clicks Play/Install on the website, which navigates to `securexe://run?repo=owner/repo&commit=<optional-sha>`.
2. The helper parses `repo`/`commit`, detects local OS/arch, and fetches the manifest (incl. `sha256`) for that build.
3. It downloads the matching artifact, verifies its `sha256` against the manifest, `chmod +x`s it (macOS/Linux), and spawns it as a subprocess.
4. Already-downloaded commits are skipped on subsequent runs.

The `securexe://` scheme deliberately carries only `repo` + optional `commit` — never a raw URL, target/arch, or host — so no webpage can turn it into an arbitrary "download and run anything from anywhere" primitive. The orchestrator base URL is pinned inside the helper.

## Out of scope for v1

Persistent library/install-list UI, background auto-update checks, run-confirmation prompts, sandboxing/permissions model.

## Status

Early design phase — no launcher code yet. Open questions: production orchestrator URL, helper tech stack (Tauri vs. Electron vs. a minimal Go binary), and the exact website → launcher handoff mechanism.
