//! What the plugin accepts from the owner for a host (plugins/ssh/docs/
//! README.md, hosts). The webview is untrusted (ADR-0004), so every command
//! checks its input with these rules before anything is stored, and the
//! plugin's UI checks the same limits only to answer early.

use std::net::{Ipv4Addr, Ipv6Addr};

/// The most characters of a host's name.
pub const MAX_NAME_CHARS: u32 = 64;

/// The most bytes of a host name in an address (RFC 1123).
const MAX_HOSTNAME_BYTES: usize = 253;

/// The most bytes of one label of a host name.
const MAX_LABEL_BYTES: usize = 63;

/// The most characters of a user name. With the longest host name and a
/// port, the endpoint stays within the 512 characters that the core takes
/// in a remote-command access.
pub const MAX_USERNAME_CHARS: u32 = 128;

/// The most bytes of a password.
pub const MAX_PASSWORD_BYTES: u32 = 1024;

/// The most characters of the owner's note about a host.
pub const MAX_NOTE_CHARS: u32 = 500;

/// The port of SSH, which the form starts with.
pub const DEFAULT_PORT: u16 = 22;

/// Whether `name` can name a host: 1 to 64 ASCII letters, digits, `.`, `_`
/// and `-`, starting with a letter or a digit. The name is also the token
/// that "@" puts into a message and the argument that the model passes, so
/// it holds nothing that a shell or Markdown would read.
pub fn is_host_name(name: &str) -> bool {
    let has_valid_length = (1..=MAX_NAME_CHARS as usize).contains(&name.len());
    let starts_well = name
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric());
    let has_valid_characters = name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'));
    has_valid_length && starts_well && has_valid_characters
}

/// The address as the plugin stores it, or `None` when `address` is not one:
/// a host name (RFC 1123), an IPv4 address, or an IPv6 address with or
/// without its brackets, which are dropped. An address never holds a scheme,
/// a user or a port.
pub fn stored_address(address: &str) -> Option<String> {
    if address.parse::<Ipv4Addr>().is_ok() {
        return Some(address.to_owned());
    }
    let unbracketed = address
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(address);
    if let Ok(ipv6) = unbracketed.parse::<Ipv6Addr>() {
        return Some(ipv6.to_string());
    }
    is_hostname(address).then(|| address.to_ascii_lowercase())
}

/// Whether `text` is a host name: labels of ASCII letters, digits and `-`,
/// which neither starts nor ends a label, joined by single dots. A name that
/// reads as a number, such as `10.0.0.300`, is refused, because the owner
/// meant an IPv4 address.
fn is_hostname(text: &str) -> bool {
    let has_valid_length = (1..=MAX_HOSTNAME_BYTES).contains(&text.len());
    let labels_are_valid = text.split('.').all(|label| {
        let has_valid_label_length = (1..=MAX_LABEL_BYTES).contains(&label.len());
        let has_valid_characters = label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
        let has_hyphen_at_an_end = label.starts_with('-') || label.ends_with('-');
        has_valid_label_length && has_valid_characters && !has_hyphen_at_an_end
    });
    let last_is_numeric = text
        .rsplit('.')
        .next()
        .is_some_and(|label| label.bytes().all(|byte| byte.is_ascii_digit()));
    has_valid_length && labels_are_valid && !last_is_numeric
}

/// Whether `port` can be a host's port.
pub fn is_port(port: u16) -> bool {
    port != 0
}

/// Whether `username` can log in: 1 to 128 characters without control
/// characters, white space or invisible characters. `@` and `\` are allowed,
/// for accounts such as `user@domain` and `DOMAIN\user` on Windows. The user
/// is part of the endpoint that approvals show, so nothing in it may change
/// how the text around it reads.
pub fn is_username(username: &str) -> bool {
    let count = username.chars().count();
    let has_valid_length = (1..=MAX_USERNAME_CHARS as usize).contains(&count);
    let has_valid_characters = username.chars().all(|character| {
        let is_hidden =
            character.is_control() || character.is_whitespace() || is_invisible(character);
        !is_hidden
    });
    has_valid_length && has_valid_characters
}

/// Characters that change how the text around them shows without showing
/// themselves, as `clean_label` removes them: the controls of
/// bidirectional text and the zero-width characters.
fn is_invisible(character: char) -> bool {
    matches!(
        character,
        '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}'
    )
}

/// Whether `password` can be a host's password: 1 to 1,024 bytes without
/// NUL, which SSH's strings could not carry to every server.
pub fn is_password(password: &str) -> bool {
    let has_valid_length = (1..=MAX_PASSWORD_BYTES as usize).contains(&password.len());
    has_valid_length && !password.contains('\0')
}

/// Whether `note` can be the owner's note about a host: at most 500
/// characters, without control characters other than line breaks.
pub fn is_note(note: &str) -> bool {
    let has_valid_length = note.chars().count() <= MAX_NOTE_CHARS as usize;
    let has_valid_characters = note
        .chars()
        .all(|character| !character.is_control() || character == '\n');
    has_valid_length && has_valid_characters
}

/// Where the host's commands run, as the owner and the policy read it:
/// `user@address:port`, with an IPv6 address in brackets.
pub fn endpoint(username: &str, address: &str, port: u16) -> String {
    if address.contains(':') {
        format!("{username}@[{address}]:{port}")
    } else {
        format!("{username}@{address}:{port}")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        endpoint, is_host_name, is_note, is_password, is_port, is_username, stored_address,
    };

    #[test]
    fn a_host_name_is_letters_digits_dots_underscores_and_hyphens_starting_with_a_letter_or_digit()
    {
        for name in ["build-box", "db01", "web.prod_2", "A", &"a".repeat(64)] {
            assert!(is_host_name(name), "{name}");
        }
        for name in [
            "",
            "-box",
            ".box",
            "_box",
            "build box",
            "box@lan",
            "box;rm",
            "bôx",
            &"a".repeat(65),
        ] {
            assert!(!is_host_name(name), "{name}");
        }
    }

    #[test]
    fn an_address_is_a_host_name_or_an_ip_address_and_nothing_more() {
        let cases = [
            ("build-box.lan", Some("build-box.lan")),
            ("Build-Box.LAN", Some("build-box.lan")),
            ("192.168.1.20", Some("192.168.1.20")),
            ("::1", Some("::1")),
            ("[fe80::1]", Some("fe80::1")),
            ("2001:DB8::0:1", Some("2001:db8::1")),
            ("", None),
            ("ssh://box", None),
            ("owner@box", None),
            ("box:22", None),
            ("[box]", None),
            ("box..lan", None),
            ("-box.lan", None),
            ("box-.lan", None),
            ("10.0.0.300", None),
            ("fe80::1%eth0", None),
            ("box lan", None),
        ];
        for (address, stored) in cases {
            assert_eq!(stored_address(address).as_deref(), stored, "{address}");
        }
        assert!(stored_address(&format!("{}.lan", "a".repeat(64))).is_none());
        let label = "a".repeat(63);
        assert!(stored_address(&[label.as_str(); 4].join(".")).is_none());
    }

    #[test]
    fn a_port_is_any_but_zero() {
        assert!(!is_port(0));
        assert!(is_port(1));
        assert!(is_port(22));
        assert!(is_port(65_535));
    }

    #[test]
    fn a_user_name_has_no_white_space_or_control_characters_and_may_name_a_domain() {
        for username in ["deploy", "user@example.org", "CORP\\admin", "jürgen"] {
            assert!(is_username(username), "{username}");
        }
        for username in [
            "",
            "two words",
            "tab\tuser",
            "line\nbreak",
            "root\u{202E}resu",
            "zero\u{200B}width",
            &"u".repeat(129),
        ] {
            assert!(!is_username(username), "{username:?}");
        }
    }

    #[test]
    fn a_password_has_up_to_1024_bytes_and_no_nul() {
        assert!(is_password("correct horse battery staple"));
        assert!(is_password(&"p".repeat(1024)));
        assert!(!is_password(""));
        assert!(!is_password("with\0nul"));
        assert!(!is_password(&"p".repeat(1025)));
    }

    #[test]
    fn a_note_keeps_line_breaks_and_refuses_other_control_characters() {
        assert!(is_note(""));
        assert!(is_note("The build machine.\nUse /srv/build."));
        assert!(is_note(&"n".repeat(500)));
        assert!(!is_note("bell\u{7}"));
        assert!(!is_note(&"n".repeat(501)));
    }

    #[test]
    fn an_endpoint_puts_an_ipv6_address_in_brackets() {
        assert_eq!(
            endpoint("deploy", "build-box.lan", 22),
            "deploy@build-box.lan:22"
        );
        assert_eq!(endpoint("root", "fe80::1", 2222), "root@[fe80::1]:2222");
    }
}
