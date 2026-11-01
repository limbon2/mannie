//! The host's password, replaced in a stream of a channel as its bytes
//! arrive (T-41, T-71). The bytes at the end of a read that could start the
//! password wait until the next read shows whether they do, so a password
//! that a read cuts is still replaced. A password shorter than
//! [`MIN_REDACTED_PASSWORD_BYTES`] is left as it is, as the log redactor
//! leaves its short secrets: it could match ordinary text.

use mannie_plugin_sdk::contracts::Secret;

use crate::commands::MIN_REDACTED_PASSWORD_BYTES;
use crate::output::REDACTED;

/// Replaces the host's password in a stream as its bytes arrive.
#[derive(Debug)]
pub(crate) struct Redaction {
    /// The password, or `None` when it is too short to tell apart from
    /// ordinary text, as for `run_ssh_command`.
    password: Option<Secret>,
    /// The bytes at the end of the last read that could start the password.
    carry: Vec<u8>,
}

impl Redaction {
    /// The redaction of a stream on a host whose password is `password`.
    pub(crate) fn new(password: Secret) -> Self {
        let is_long_enough = password.expose().len() >= MIN_REDACTED_PASSWORD_BYTES as usize;
        Self {
            password: is_long_enough.then_some(password),
            carry: Vec::new(),
        }
    }

    /// The bytes of a read that are known, with the password replaced.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<u8> {
        let Some(password) = &self.password else {
            return bytes.to_vec();
        };
        let password = password.expose().as_bytes();
        let mut input = std::mem::take(&mut self.carry);
        input.extend_from_slice(bytes);
        let mut output = Vec::with_capacity(input.len());
        let mut rest = input.as_slice();
        while let Some(at) = find(rest, password) {
            output.extend_from_slice(&rest[..at]);
            output.extend_from_slice(REDACTED.as_bytes());
            rest = &rest[at + password.len()..];
        }
        let known = rest.len() - held_back(rest, password);
        output.extend_from_slice(&rest[..known]);
        self.carry = rest[known..].to_vec();
        output
    }

    /// The bytes kept back, which did not become the password, at the
    /// stream's end or once the stream paused.
    pub(crate) fn finish(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.carry)
    }

    /// Whether bytes are kept back.
    pub(crate) fn holds_back(&self) -> bool {
        !self.carry.is_empty()
    }
}

/// Where `needle` first occurs in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// How many bytes at the end of `bytes` could start the password.
fn held_back(bytes: &[u8], password: &[u8]) -> usize {
    (1..password.len())
        .rev()
        .find(|&length| bytes.ends_with(&password[..length]))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::Secret;

    use super::Redaction;

    #[test]
    fn a_cut_password_is_replaced_and_what_only_starts_it_waits() {
        let mut redaction = Redaction::new(Secret::new("correct horse battery".to_owned()));

        let first = redaction.push(b"say correct h");
        let holding = redaction.holds_back();
        let second = redaction.push(b"orse battery and correct");
        let rest = redaction.finish();

        assert_eq!(first, b"say ");
        assert!(holding);
        assert_eq!(second, b"[REDACTED] and ");
        assert_eq!(rest, b"correct");
        assert!(!redaction.holds_back());
    }
}
