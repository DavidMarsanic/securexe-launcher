# Security Policy

brightencode-launcher is a trust-sensitive component: it registers the
`securexe://` protocol, verifies Ed25519-signed launch links, and downloads
and runs binaries on the user's machine. Please report vulnerabilities
privately rather than as a public issue.

## Reporting a vulnerability

Preferred: open a [GitHub Security Advisory](../../security/advisories/new)
for this repo (private by default — only visible to maintainers until a fix
ships).

Alternative: email marsanic.david@gmail.com.

Please include what you found, the impact, and steps to reproduce. This is a
small, independently-maintained project without a bug bounty program, but
real reports will be acknowledged and fixed as quickly as possible, with
credit given (unless you'd prefer otherwise) once a fix is out.

## Scope

In scope: the launcher's link-verification logic (`src-tauri/src/signature.rs`),
download/checksum verification (`src-tauri/src/verify.rs`), and anything that
could let a malicious webpage or a compromised download get code running
without the checks described in the [README](README.md#link-signing).

Out of scope: the brightencode-web site and the build orchestrator — those live
in separate, private repos; please report issues with them directly to
marsanic.david@gmail.com as well.

## Verifying releases

See [Verifying your download](README.md#verifying-your-download) in the
README for how to check a release's checksum and GitHub build attestation
before running it.
