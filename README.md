# securexe-launcher

A small, cross-platform desktop helper — the "install + launch" half of the Securexe platform. It does not browse, build, or manage a catalog; that lives on the website. Its job is narrow: register the `securexe://` URL protocol, and when invoked, download the right binary from the orchestrator at `worker.brightencode.com` and run it.

Built with [Tauri](https://tauri.app) (Rust core + a plain HTML/JS/CSS frontend, no framework) — small binary, cross-platform, and this v1 plumbing is meant to become the core of a future full "Steam-like" client.

## How it works (v1)

1. User clicks Play/Install on the website, which navigates to `securexe://run?repo=owner/repo&commit=<optional-sha>`.
2. The helper parses `repo`/`commit`, detects local OS/arch, and fetches the manifest (incl. `sha256`) for that build from the orchestrator.
3. It downloads the matching artifact, verifies its `sha256` against the manifest, `chmod +x`s it (macOS/Linux), and spawns it as a subprocess.
4. Already-downloaded commits are skipped on subsequent runs (cached under `~/.securexe/apps/<owner__repo>/<commit>/`).

The `securexe://` scheme deliberately carries only `repo` + optional `commit` — never a raw URL, target/arch, or host — so no webpage can turn it into an arbitrary "download and run anything from anywhere" primitive. The orchestrator base URL (`https://worker.brightencode.com`) is pinned inside the helper.

## Out of scope for v1

Persistent library/install-list UI, background auto-update checks, run-confirmation prompts, sandboxing/permissions model.

## Development

```
cargo tauri dev
```

Note: `cargo tauri dev` runs the raw debug binary, which is not a registered `.app` bundle, so macOS won't route `securexe://` links to it. To test the actual protocol handoff, build and open the bundle once:

```
cargo tauri build --debug
open "src-tauri/target/debug/bundle/macos/Securexe Launcher.app"
open "securexe://run?repo=gohugoio/hugo"   # a real repo already built on the live orchestrator
```

## Releasing

Push a tag (`git tag v0.1.0 && git push origin v0.1.0`) or run the "Release" workflow manually from the Actions tab. CI builds installers for macOS (arm64 + x64), Windows, and Linux, and attaches them to a **draft** GitHub Release under both their versioned name and a fixed name (e.g. `securexe-launcher-macos-arm64.dmg`) that never changes between releases.

The release stays a draft — nothing is publicly downloadable — until you manually click "Publish" on it. Once published, link to installers with the stable, version-agnostic URL pattern so the website link never needs updating:

```
https://github.com/DavidMarsanic/securexe-launcher/releases/latest/download/securexe-launcher-macos-arm64.dmg
```

macOS builds are not yet code-signed/notarized — see the Status section below.

## Status

v1 flow implemented and verified end-to-end against the live production orchestrator (`gohugoio/hugo`): protocol registration, manifest fetch, download, sha256 verification, cache-hit skip, and execution all work. Not yet packaged/signed for distribution.
