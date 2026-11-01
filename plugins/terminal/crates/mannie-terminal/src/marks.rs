//! The markers that a shell's setup writes around its prompts and commands
//! (plugins/terminal/docs/adr/0001-mark-commands-with-shell-integration.md):
//! OSC 633 sequences, `ESC ] 633 ; <kind> [; <payload>] ; <nonce> BEL`,
//! whose last parameter is the terminal's nonce.
//!
//! Output can forge a marker's form, but not the nonce, which only the setup
//! knows, so a marker without it is plain output (T-69). Values in a payload
//! escape `\` as `\\` and `;` as `\x3b`, and may escape any byte as `\xHH`.

use std::fmt::Write as _;

/// The kind of OSC that carries markers.
const MARKER_OSC: &[u8] = b"633";

/// The most bytes of a marker's payload: a command line or a property.
const MAX_PAYLOAD_BYTES: usize = 4096;

/// A terminal's nonce: 32 hexadecimal digits from the system's random
/// source, or `None` when the system gives no random bytes. A nonce is a
/// secret, not an ID, so all of its bits are random.
pub(crate) fn new_nonce() -> Option<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).ok()?;
    let mut nonce = String::with_capacity(2 * bytes.len());
    for byte in bytes {
        // Writing to a string cannot fail.
        let _written = write!(nonce, "{byte:02x}");
    }
    Some(nonce)
}

/// What a marker says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mark {
    /// The prompt starts (`A`).
    PromptStart,
    /// The prompt ends, and the command line starts (`B`).
    PromptEnd,
    /// The command line was read, and the command's output starts (`C`).
    CommandStart,
    /// The command ended, with its exit code when the shell gives one (`D`).
    CommandEnd {
        /// The exit code.
        exit: Option<i32>,
    },
    /// The command line, as the shell read it (`E`).
    CommandLine(String),
    /// The folder that the shell is in (`P;Cwd=`).
    Folder(String),
    /// The user that the shell runs as (`P;User=`).
    User(String),
}

/// The marker of an OSC's parameters, or `None` for an OSC that is not a
/// marker of this terminal: another kind, another nonce, an unknown marker
/// or a payload that breaks the rules.
pub(crate) fn parse(params: &[&[u8]], nonce: &[u8]) -> Option<Mark> {
    let (first, rest) = params.split_first()?;
    if *first != MARKER_OSC {
        return None;
    }
    let (last, middle) = rest.split_last()?;
    if *last != nonce {
        return None;
    }
    let (kind, payload) = middle.split_first()?;
    // A payload that held `;` without escaping it arrives in pieces.
    let payload = payload.join(&b';');
    if payload.len() > MAX_PAYLOAD_BYTES {
        return None;
    }
    match *kind {
        b"A" if payload.is_empty() => Some(Mark::PromptStart),
        b"B" if payload.is_empty() => Some(Mark::PromptEnd),
        b"C" if payload.is_empty() => Some(Mark::CommandStart),
        b"D" => Some(Mark::CommandEnd {
            exit: exit_code(&payload),
        }),
        b"E" => unescape(&payload).map(Mark::CommandLine),
        b"P" => property(&payload),
        _ => None,
    }
}

/// The exit code of a `D` marker: a decimal integer, or nothing.
fn exit_code(payload: &[u8]) -> Option<i32> {
    std::str::from_utf8(payload).ok()?.trim().parse().ok()
}

/// A `P` marker's property: `Cwd=<folder>` or `User=<name>`, not empty.
fn property(payload: &[u8]) -> Option<Mark> {
    let separator = payload.iter().position(|byte| *byte == b'=')?;
    let (name, value) = payload.split_at(separator);
    let value = unescape(&value[1..])?;
    if value.trim().is_empty() {
        return None;
    }
    match name {
        b"Cwd" => Some(Mark::Folder(value)),
        b"User" => Some(Mark::User(value)),
        _ => None,
    }
}

/// A payload's text without its escapes, or `None` for text that is not
/// UTF-8. A backslash that starts no escape stays, since cmd cannot escape
/// the backslashes of its paths. Control characters other than tabs become
/// spaces, so a value never moves the cursor where it shows.
fn unescape(payload: &[u8]) -> Option<String> {
    let mut bytes = Vec::with_capacity(payload.len());
    let mut index = 0;
    while index < payload.len() {
        let byte = payload[index];
        if byte != b'\\' {
            bytes.push(byte);
            index += 1;
            continue;
        }
        if payload.get(index + 1) == Some(&b'\\') {
            bytes.push(b'\\');
            index += 2;
            continue;
        }
        let escaped = (payload.get(index + 1) == Some(&b'x'))
            .then(|| payload.get(index + 2..index + 4))
            .flatten()
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| u8::from_str_radix(digits, 16).ok());
        if let Some(value) = escaped {
            bytes.push(value);
            index += 4;
        } else {
            bytes.push(b'\\');
            index += 1;
        }
    }
    let text = String::from_utf8(bytes).ok()?;
    Some(
        text.chars()
            .map(|character| {
                if character.is_control() && character != '\t' {
                    ' '
                } else {
                    character
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::{Mark, new_nonce, parse};

    const NONCE: &[u8] = b"0123456789abcdef0123456789abcdef";

    fn marker(parts: &[&str]) -> Option<Mark> {
        let mut params: Vec<&[u8]> = vec![b"633"];
        params.extend(parts.iter().map(|part| part.as_bytes()));
        params.push(NONCE);
        parse(&params, NONCE)
    }

    #[test]
    fn markers_with_the_nonce_say_what_happened() {
        assert_eq!(marker(&["A"]), Some(Mark::PromptStart));
        assert_eq!(marker(&["B"]), Some(Mark::PromptEnd));
        assert_eq!(marker(&["C"]), Some(Mark::CommandStart));
        assert_eq!(
            marker(&["D", "130"]),
            Some(Mark::CommandEnd { exit: Some(130) })
        );
        assert_eq!(marker(&["D", ""]), Some(Mark::CommandEnd { exit: None }));
        assert_eq!(marker(&["D"]), Some(Mark::CommandEnd { exit: None }));
        assert_eq!(
            marker(&["P", r"Cwd=C:\\Users\\me\x3bwork"]),
            Some(Mark::Folder(r"C:\Users\me;work".to_owned()))
        );
        assert_eq!(
            marker(&["P", "User=root"]),
            Some(Mark::User("root".to_owned()))
        );
        // cmd cannot escape its paths, whose backslashes stay.
        assert_eq!(
            marker(&["P", r"Cwd=C:\Users\me"]),
            Some(Mark::Folder(r"C:\Users\me".to_owned()))
        );
        assert_eq!(
            marker(&["P", r"Cwd=\x4"]),
            Some(Mark::Folder(r"\x4".to_owned()))
        );
        // A command line that kept its `;` arrives in pieces.
        assert_eq!(
            marker(&["E", "cd /tmp", " ls"]),
            Some(Mark::CommandLine("cd /tmp; ls".to_owned()))
        );
    }

    #[test]
    fn a_forged_or_broken_marker_is_plain_output() {
        let forged: Vec<&[u8]> = vec![b"633", b"D", b"0", b"guessed"];
        assert_eq!(parse(&forged, NONCE), None);
        let other_osc: Vec<&[u8]> = vec![b"0", b"title", NONCE];
        assert_eq!(parse(&other_osc, NONCE), None);
        assert_eq!(marker(&["Z"]), None);
        assert_eq!(marker(&["A", "extra"]), None);
        assert_eq!(marker(&["P", "Cwd="]), None);
        assert_eq!(marker(&["P", "Shell=bash"]), None);
        let long = "x".repeat(5000);
        assert_eq!(marker(&["E", &long]), None);
    }

    #[test]
    fn a_value_never_carries_control_characters() {
        assert_eq!(
            marker(&["E", r"echo \x1b[2Jgone"]),
            Some(Mark::CommandLine("echo  [2Jgone".to_owned()))
        );
    }

    #[test]
    fn each_terminal_gets_its_own_nonce() {
        let first = new_nonce().unwrap();
        assert_eq!(first.len(), 32);
        assert!(first.chars().all(|character| character.is_ascii_hexdigit()));
        assert_ne!(first, new_nonce().unwrap());
    }
}
