//! SHA-256 digests: of an operation's accesses, which an approval request
//! carries and a decision must name (T-5), and of a file's bytes, which is the
//! file's revision.

use mannie_types::tools::Access;
use sha2::{Digest, Sha256};

/// What the JSON of a command's access in the foreground ends with: its last
/// field, `background`, false. The digest leaves it out, so a command in the
/// foreground keeps the digest that it had before commands ran in the
/// background (ADR-0041, decision 2). Inside a JSON string every quote is
/// escaped, so the sequence can only be the field itself.
const FOREGROUND_FIELD: &[u8] = b",\"background\":false}";

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
    hexadecimal(&Sha256::digest(canonical(&json)))
}

/// The JSON of accesses without the field of a command in the foreground.
fn canonical(json: &[u8]) -> Vec<u8> {
    let mut kept = Vec::with_capacity(json.len());
    let mut rest = json;
    while let Some(at) = rest
        .windows(FOREGROUND_FIELD.len())
        .position(|window| window == FOREGROUND_FIELD)
    {
        kept.extend_from_slice(&rest[..at]);
        kept.push(b'}');
        rest = &rest[at + FOREGROUND_FIELD.len()..];
    }
    kept.extend_from_slice(rest);
    kept
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
    use sha2::Digest as _;

    use super::{digest, revision};

    fn write(identity: &str) -> Access {
        with_state(identity, None)
    }

    fn with_state(identity: &str, state: Option<&str>) -> Access {
        Access::Path {
            path: "/work/notes.txt".to_owned(),
            mode: AccessMode::Write,
            folder: false,
            exists: true,
            identity: identity.to_owned(),
            entries: None,
            state: state.map(str::to_owned),
        }
    }

    #[test]
    fn a_plugin_s_state_of_a_path_changes_the_digest() {
        assert_ne!(
            digest(&[with_state("2049:12", Some("HEAD 1a"))]),
            digest(&[with_state("2049:12", Some("HEAD 2b"))])
        );
        assert_ne!(
            digest(&[with_state("2049:12", Some("HEAD 1a"))]),
            digest(&[write("2049:12")])
        );
    }

    fn command(text: &str, background: bool) -> Access {
        Access::Command {
            command: text.to_owned(),
            shell: "/bin/sh".to_owned(),
            working_directory: "/work".to_owned(),
            background,
        }
    }

    /// The SHA-256 of `json`, in hexadecimal.
    fn sha256(json: &str) -> String {
        super::hexadecimal(&sha2::Sha256::digest(json.as_bytes()))
    }

    #[test]
    fn a_command_in_the_foreground_keeps_the_digest_of_an_access_without_the_mode() {
        let before =
            r#"[{"kind":"command","command":"ls","shell":"/bin/sh","workingDirectory":"/work"}]"#;
        let tricky = r#"[{"kind":"command","command":"echo ,\"background\":false}","shell":"/bin/sh","workingDirectory":"/work"}]"#;

        assert_eq!(digest(&[command("ls", false)]), sha256(before));
        assert_ne!(
            digest(&[command("ls", true)]),
            digest(&[command("ls", false)])
        );
        // The field's text inside a command is escaped, and stays.
        assert_eq!(
            digest(&[command(r#"echo ,"background":false}"#, false)]),
            sha256(tricky)
        );
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
