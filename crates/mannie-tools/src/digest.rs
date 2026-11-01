//! SHA-256 digests: of an operation's accesses, which an approval request
//! carries and a decision must name (T-5), of a write to a registered
//! database, which a worker computes again (ADR-0057, decision 7), and of a
//! file's bytes, which is the file's revision.

use mannie_types::ids::DatabaseKey;
use mannie_types::tools::{Access, StatementClass};
use serde::Serialize;
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

/// One statement of a write to a registered database, as the write's
/// digest names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WrittenStatement<'a> {
    /// The statement's text.
    pub text: &'a str,
    /// The digest of its parameters' values.
    pub parameters_digest: &'a str,
    /// What it does, as SQLite reports it.
    pub class: StatementClass,
}

/// The digest of a write to a registered database: the SHA-256 of the
/// database's key, the file's identity and the write's statements in JSON,
/// in hexadecimal. The owner's Allow once of a write on a worker's database
/// names it, and the worker computes it again from its own registry and the
/// write, so a statement, a value, a class or a file that differs from what
/// the owner approved goes back to the owner (ADR-0057, decision 7).
///
/// # Panics
///
/// Never: a write holds only strings, which always serialize.
pub fn write_digest(
    key: &DatabaseKey,
    identity: &str,
    statements: &[WrittenStatement<'_>],
) -> String {
    #[derive(Serialize)]
    struct Write<'a> {
        key: &'a DatabaseKey,
        identity: &'a str,
        statements: &'a [WrittenStatement<'a>],
    }
    let json = serde_json::to_vec(&Write {
        key,
        identity,
        statements,
    })
    .expect("a write holds only strings, which always serialize");
    hexadecimal(&Sha256::digest(json))
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
    use mannie_types::ids::DatabaseKey;
    use mannie_types::tools::{Access, AccessMode, StatementClass};
    use sha2::Digest as _;

    use super::{WrittenStatement, digest, revision, write_digest};

    #[test]
    fn a_writes_digest_covers_its_database_its_file_and_each_statement() {
        let key = DatabaseKey::parse("0199e0f2-0000-7000-8000-0000000000ee").unwrap();
        let statement =
            |text: &'static str, parameters_digest: &'static str, class| WrittenStatement {
                text,
                parameters_digest,
                class,
            };
        let update = statement("UPDATE parts SET stock = ?1", "aa", StatementClass::Write);
        let shown = write_digest(&key, "7:42", &[update]);

        assert_eq!(shown, write_digest(&key, "7:42", &[update]));
        assert_eq!(shown.len(), 64);
        let other_key = DatabaseKey::parse("0199e0f2-0000-7000-8000-0000000000ef").unwrap();
        for other in [
            write_digest(&other_key, "7:42", &[update]),
            write_digest(&key, "7:43", &[update]),
            write_digest(
                &key,
                "7:42",
                &[statement(
                    "UPDATE parts SET stock = 1",
                    "aa",
                    StatementClass::Write,
                )],
            ),
            write_digest(
                &key,
                "7:42",
                &[statement(
                    "UPDATE parts SET stock = ?1",
                    "ab",
                    StatementClass::Write,
                )],
            ),
            write_digest(
                &key,
                "7:42",
                &[statement(
                    "UPDATE parts SET stock = ?1",
                    "aa",
                    StatementClass::Schema,
                )],
            ),
            write_digest(&key, "7:42", &[update, update]),
        ] {
            assert_ne!(shown, other);
        }
    }

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
