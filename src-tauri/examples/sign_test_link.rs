//! Dev/testing utility that signs a `securexe://run` link the same way the
//! website's backend is expected to. Never hardcode the private key here —
//! it's read from the `SIGNING_PRIVATE_KEY_HEX` env var so this file stays
//! safe to commit.
//!
//! Usage:
//!   SIGNING_PRIVATE_KEY_HEX=<hex> cargo run --example sign_test_link -- owner/repo [commit]
use ed25519_dalek::{Signer, SigningKey};
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let key_hex = std::env::var("SIGNING_PRIVATE_KEY_HEX")
        .expect("set SIGNING_PRIVATE_KEY_HEX to the private key hex (never commit it)");
    let key_bytes: [u8; 32] = hex::decode(key_hex.trim())
        .expect("invalid hex")
        .try_into()
        .expect("private key must be 32 bytes");
    let signing_key = SigningKey::from_bytes(&key_bytes);

    let mut args = std::env::args().skip(1);
    let repo = args.next().expect("usage: sign_test_link <owner/repo> [commit]");
    let commit = args.next();

    let exp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;

    let message = format!("{repo}|{}|{exp}", commit.as_deref().unwrap_or(""));
    let signature = signing_key.sign(message.as_bytes());
    let sig_hex = hex::encode(signature.to_bytes());

    let mut url = format!("securexe://run?repo={repo}&exp={exp}&sig={sig_hex}");
    if let Some(c) = commit {
        url.push_str(&format!("&commit={c}"));
    }
    println!("{url}");
}
