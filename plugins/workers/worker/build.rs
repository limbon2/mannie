//! Embeds the release key (docs/engineering/releasing.md, signing): the raw
//! Ed25519 public key from `release/mannie-release.pub`, which `mannie-worker
//! update` checks a release archive's signature with. A key that is not an
//! Ed25519 public key in PEM fails the build, so no worker is built without
//! the release key.

use std::error::Error;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::{env, fs};

use base64::Engine as _;

/// The DER of an Ed25519 `SubjectPublicKeyInfo` before its 32 key bytes
/// (RFC 8410): a sequence, the algorithm identifier 1.3.101.112, and a bit
/// string of 33 bytes with no unused bits.
const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

fn main() -> Result<(), Box<dyn Error>> {
    let key_file =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("../../../release/mannie-release.pub");
    println!("cargo::rerun-if-changed={}", key_file.display());
    let pem = fs::read_to_string(&key_file).map_err(|error| {
        format!(
            "could not read the release key {}: {error}",
            key_file.display()
        )
    })?;
    let spki = spki_of(&pem).map_err(|problem| {
        format!(
            "the release key {} is not valid: {problem}",
            key_file.display()
        )
    })?;
    let is_ed25519 =
        spki.len() == ED25519_SPKI_PREFIX.len() + 32 && spki.starts_with(&ED25519_SPKI_PREFIX);
    if !is_ed25519 {
        return Err(format!(
            "the release key {} is not an Ed25519 public key",
            key_file.display()
        )
        .into());
    }
    let source = format!(
        "/// The release key's `SubjectPublicKeyInfo`, whose last 32 bytes are the\n\
         /// raw Ed25519 key.\n\
         pub(crate) const RELEASE_KEY_SPKI: [u8; 44] = [{}];\n",
        byte_list(&spki)?
    );
    fs::write(
        PathBuf::from(env::var("OUT_DIR")?).join("release_key.rs"),
        source,
    )?;
    Ok(())
}

/// The DER of a PEM `PUBLIC KEY` block.
fn spki_of(pem: &str) -> Result<Vec<u8>, String> {
    let body: String = pem
        .lines()
        .map(str::trim)
        .skip_while(|line| *line != "-----BEGIN PUBLIC KEY-----")
        .skip(1)
        .take_while(|line| *line != "-----END PUBLIC KEY-----")
        .collect();
    if body.is_empty() {
        return Err("it has no PUBLIC KEY block".to_owned());
    }
    base64::engine::general_purpose::STANDARD
        .decode(body)
        .map_err(|error| format!("its PUBLIC KEY block is not base64: {error}"))
}

/// Bytes as a Rust array's elements.
fn byte_list(bytes: &[u8]) -> Result<String, std::fmt::Error> {
    let mut list = String::new();
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 {
            list.push_str(", ");
        }
        write!(list, "0x{byte:02x}")?;
    }
    Ok(list)
}
