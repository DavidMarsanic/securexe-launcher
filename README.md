# securexe-launcher

A small, cross-platform desktop helper — the "install + launch" half of the Securexe platform. It does not browse, build, or manage a catalog; that lives on the website. Its job is narrow: register the `securexe://` URL protocol, and when invoked, download the right binary from the orchestrator at `worker.brightencode.com` and run it.

Built with [Tauri](https://tauri.app) (Rust core + a plain HTML/JS/CSS frontend, no framework) — small binary, cross-platform, and this v1 plumbing is meant to become the core of a future full "Steam-like" client.

## How it works (v1)

1. The website signs a link server-side and navigates to `securexe://run?repo=owner/repo&commit=<optional-sha>&exp=<unix-ts>&sig=<hex>`.
2. The helper verifies `sig` against an Ed25519 public key embedded in the binary before doing anything else — see "Link signing" below. Unsigned, expired, or tampered links are rejected with no network activity at all.
3. It parses `repo`/`commit`, detects local OS/arch, and fetches the manifest (incl. `sha256`) for that build from the orchestrator.
4. It downloads the matching artifact, verifies its `sha256` against the manifest, `chmod +x`s it (macOS/Linux), and spawns it as a subprocess.
5. Already-downloaded commits are skipped on subsequent runs (cached under `~/.securexe/apps/<owner__repo>/<commit>/`).

The `securexe://` scheme deliberately carries only `repo` + optional `commit` — never a raw URL, target/arch, or host — so no webpage can turn it into an arbitrary "download and run anything from anywhere" primitive. The orchestrator base URL (`https://worker.brightencode.com`) is pinned inside the helper.

## Link signing

`securexe://` is a globally registered OS protocol handler — once installed, *any* webpage could construct a `securexe://run?repo=...` link, not just brightencode.com's. The OS gives the launcher no way to know which site a click came from, so instead every link must be signed with an Ed25519 private key that only the website's backend holds; the launcher only embeds the corresponding **public** key (`src-tauri/src/signature.rs`), which can verify signatures but never create them.

Signed message format (must match exactly on both sides): `"{owner/repo}|{commit or empty string}|{exp}"`, where `exp` is a Unix timestamp the launcher rejects if already passed or more than 300 seconds in the future — links are meant to be used immediately, not bookmarked or shared long-term.

- Generate a keypair: `cargo run --example gen_signing_key` (in `src-tauri/`). The private key must only ever live server-side (e.g. an env var on the website) — never commit it, never put it in this repo.
- `cargo run --example sign_test_link -- owner/repo [commit]` (reads the private key from `SIGNING_PRIVATE_KEY_HEX` env var) is a dev tool for generating test links and a reference implementation of the exact signing scheme the website needs to replicate.

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
