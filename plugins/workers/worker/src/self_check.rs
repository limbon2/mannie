//! `self-check --json` (plugins/workers/docs/compatibility-and-updates.md):
//! the program's version, system and release key. `update` reads a new
//! program's check before it installs it, and the release workflow compares
//! the key with the release key, so the output keeps its shape in every
//! version.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

include!(concat!(env!("OUT_DIR"), "/release_key.rs"));

/// The length of the DER before the raw key in [`RELEASE_KEY_SPKI`].
const SPKI_PREFIX_BYTES: usize = 12;

/// What a program says about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SelfCheck {
    /// The program's version.
    pub(crate) version: String,
    /// The system it runs on: `windows`, `linux` or `freebsd`.
    pub(crate) os: String,
    /// The processor: `x86_64` or `aarch64`.
    pub(crate) arch: String,
    /// The SHA-256 of the release key's `SubjectPublicKeyInfo`, in hexadecimal.
    pub(crate) release_key_sha256: String,
}

/// The raw Ed25519 release key that update checks signatures with.
pub(crate) fn release_key() -> &'static [u8] {
    &RELEASE_KEY_SPKI[SPKI_PREFIX_BYTES..]
}

/// This program's check.
pub(crate) fn this_program() -> SelfCheck {
    let digest = ring::digest::digest(&ring::digest::SHA256, &RELEASE_KEY_SPKI);
    let mut fingerprint = String::with_capacity(64);
    for byte in digest.as_ref() {
        // Writing to a String cannot fail.
        let _written = write!(fingerprint, "{byte:02x}");
    }
    SelfCheck {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        release_key_sha256: fingerprint,
    }
}

/// Prints this program's check as one line of JSON.
pub(crate) fn print() -> anyhow::Result<()> {
    let line = serde_json::to_string(&this_program())?;
    #[expect(
        clippy::print_stdout,
        reason = "self-check writes its JSON on standard output"
    )]
    {
        println!("{line}");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
