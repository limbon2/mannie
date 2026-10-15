//! SHA-256 digests: of an operation's accesses, which an approval request
//! carries and a decision must name (T-5), and of a file's bytes, which is the
//! file's revision.

use mannie_types::tools::Access;
use sha2::{Digest, Sha256};

/// The digest of an operation: the SHA-256 of its accesses in canonical JSON,
/// in hexadecimal. A replaced file, a swapped folder or a changed folder
/// content changes it.
///
/// # Panics
///
/// Never: accesses hold only strings, booleans and numbers, which always
/// serialize.
pub fn digest(accesses: &[Access]) -> String {
    let json = serde_json::to_vec(accesses)
        .expect("accesses hold only strings, booleans and numbers, which always serialize");
    hexadecimal(&Sha256::digest(&json))
}

/// The revision of a file's bytes: `sha256:` and the hexadecimal digest.
pub(crate) fn revision(bytes: &[u8]) -> String {
    format!("sha256:{}", hexadecimal(&Sha256::digest(bytes)))
}

/// Bytes in lower-case hexadecimal.
fn hexadecimal(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(hex_digit(byte >> 4));
        text.push(hex_digit(byte & 0x0f));
    }
    text
}

fn hex_digit(value: u8) -> char {
    char::from_digit(u32::from(value), 16).unwrap_or('0')
}

#[cfg(test)]
mod tests {
    use mannie_types::tools::{Access, AccessMode};

    use super::{digest, revision};

    fn write(identity: &str) -> Access {
        Access::Path {
            path: "/work/notes.txt".to_owned(),
            mode: AccessMode::Write,
            folder: false,
            exists: true,
            identity: identity.to_owned(),
            entries: None,
        }
    }

    #[test]
    fn a_changed_identity_changes_the_digest() {
        assert_eq!(digest(&[write("2049:12")]), digest(&[write("2049:12")]));
        assert_ne!(digest(&[write("2049:12")]), digest(&[write("2049:13")]));
        assert_eq!(digest(&[write("2049:12")]).len(), 64);
    }

    #[test]
    fn revisions_name_their_algorithm() {
        assert_eq!(
            revision(b"abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
