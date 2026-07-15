//! Dev utility to (re)generate the Ed25519 keypair used to authorize
//! `securexe://run` links. Run with `cargo run --example gen_signing_key`.
//!
//! The private key must only ever live server-side on the website that
//! signs links (never in this repo, never in the launcher binary). The
//! public key is safe to commit — it can verify signatures but not create
//! them — and belongs in `src-tauri/src/signature.rs`.
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;

fn main() {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    println!(
        "PRIVATE KEY (server-side only, never commit): {}",
        hex::encode(signing_key.to_bytes())
    );
    println!(
        "PUBLIC KEY  (embed in signature.rs, safe to commit): {}",
        hex::encode(verifying_key.to_bytes())
    );
}
