use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::LauncherError;

/// Public half of the brightencode.com website's signing key. Safe to embed
/// here — this side can only *verify* signatures, never create them. Only
/// whoever holds the matching private key (kept server-side on the website,
/// generated via `cargo run --example gen_signing_key`) can mint a link this
/// launcher will accept. This is what makes `securexe://run` links usable
/// only by that backend, rather than by any webpage that copies the URL
/// shape.
const PUBLIC_KEY_BYTES: [u8; 32] = [
    0xf0, 0x87, 0x80, 0xfd, 0xa7, 0xa0, 0x45, 0x8e, 0x7d, 0x51, 0x0e, 0xb0, 0x8c, 0xb1, 0xd4, 0x01,
    0x51, 0xf6, 0x15, 0xac, 0xfd, 0x0d, 0xc4, 0x5c, 0x9b, 0xbf, 0x26, 0x21, 0xb8, 0xed, 0xff, 0xd4,
];

/// Signed links are only valid for this long after the `exp` timestamp is
/// issued — bounds the damage if a link is ever captured and replayed, even
/// though the signature itself never expires cryptographically.
const MAX_VALIDITY_SECONDS: u64 = 300;

/// Reconstructs the exact message the website is expected to have signed
/// and verifies `sig_hex` against it. `commit` uses `""` (not `"none"` or
/// similar) when absent, matching whatever the signer used — this format is
/// a contract between this code and the website's signing code, not
/// something either side can change unilaterally.
pub fn verify(
    repo_path: &str,
    commit: Option<&str>,
    exp: &str,
    sig_hex: &str,
) -> Result<(), LauncherError> {
    let exp_ts: u64 = exp
        .parse()
        .map_err(|_| LauncherError::Unauthorized("invalid exp".into()))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if exp_ts < now {
        return Err(LauncherError::Unauthorized("link has expired".into()));
    }
    if exp_ts - now > MAX_VALIDITY_SECONDS {
        return Err(LauncherError::Unauthorized(
            "link validity window is too long".into(),
        ));
    }

    let message = format!("{repo_path}|{}|{exp}", commit.unwrap_or(""));

    let sig_bytes = hex::decode(sig_hex)
        .map_err(|_| LauncherError::Unauthorized("malformed signature".into()))?;
    let sig_array: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| LauncherError::Unauthorized("malformed signature".into()))?;
    let signature = Signature::from_bytes(&sig_array);

    let verifying_key = VerifyingKey::from_bytes(&PUBLIC_KEY_BYTES)
        .map_err(|_| LauncherError::Unauthorized("invalid embedded public key".into()))?;

    verifying_key
        .verify(message.as_bytes(), &signature)
        .map_err(|_| LauncherError::Unauthorized("invalid signature".into()))
}
