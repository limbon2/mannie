//! Pairing codes and proofs (docs/features/workers/pairing-and-link.md,
//! pairing).
//!
//! A code is 20 characters of Crockford base32, 100 bits: whoever captures
//! one exchange can test codes offline, so a code must be too long to guess.
//! A proof is the HMAC-SHA256, keyed by the code, of a label, both nonces and
//! both pins. Each side takes the pins from the TLS session it is in, so a
//! proof made for another session or another key never verifies.

use std::fmt;
use std::time::Duration;

use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroizing;

use crate::keys::{KeyError, KeyPin};

/// Crockford's base32 alphabet: digits and letters without I, L, O and U.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// The characters of a code.
pub const CODE_CHARS: usize = 20;

/// How long a code is valid.
pub const CODE_LIFETIME: Duration = Duration::from_mins(10);

/// How many attempts a code allows.
pub const CODE_ATTEMPTS: u32 = 5;

/// The bytes of a nonce.
pub const NONCE_BYTES: usize = 32;

/// The label of the worker's proof.
const WORKER_LABEL: &[u8] = b"mannie-pairing-v1 worker";

/// The label of the master's proof.
const MASTER_LABEL: &[u8] = b"mannie-pairing-v1 master";

/// A pairing code, in its canonical form: 20 characters of the alphabet.
/// It is a secret while it is valid, so its memory is cleared when it is
/// dropped and its debug form hides it.
#[derive(Clone, PartialEq, Eq)]
pub struct Code(Zeroizing<String>);

impl fmt::Debug for Code {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Code(..)")
    }
}

impl Code {
    /// A new random code.
    pub fn generate() -> Result<Self, KeyError> {
        let mut bytes = Zeroizing::new([0_u8; 13]);
        SystemRandom::new()
            .fill(bytes.as_mut())
            .map_err(|_unspecified| KeyError::Random)?;
        let mut code = Zeroizing::new(String::with_capacity(CODE_CHARS));
        for index in 0..CODE_CHARS {
            // The 5 bits of the character, from the first 100 bits.
            let bit = index * 5;
            let pair = (u16::from(bytes[bit / 8]) << 8) | u16::from(bytes[bit / 8 + 1]);
            let value = (pair >> (11 - bit % 8)) & 0b1_1111;
            code.push(char::from(ALPHABET[usize::from(value)]));
        }
        Ok(Self(code))
    }

    /// A code as the owner typed it: case, spaces and hyphens do not matter,
    /// and `I` and `L` read as `1` and `O` as `0`. `None` when it is not 20
    /// characters of the alphabet.
    pub fn parse(text: &str) -> Option<Self> {
        let mut code = Zeroizing::new(String::with_capacity(CODE_CHARS));
        for character in text.chars() {
            let character = match character.to_ascii_uppercase() {
                ' ' | '-' => continue,
                'I' | 'L' => '1',
                'O' => '0',
                other => other,
            };
            if !character.is_ascii() || !ALPHABET.contains(&(character as u8)) {
                return None;
            }
            code.push(character);
        }
        (code.len() == CODE_CHARS).then(|| Self(code))
    }

    /// The code as the owner reads it: five groups of four characters.
    pub fn display(&self) -> String {
        let characters: Vec<char> = self.0.chars().collect();
        characters
            .chunks(4)
            .map(|group| group.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("-")
    }

    /// The code in its canonical form, to store on the host.
    pub fn canonical(&self) -> &str {
        &self.0
    }

    fn key(&self) -> hmac::Key {
        hmac::Key::new(hmac::HMAC_SHA256, self.0.as_bytes())
    }
}

/// A new random nonce.
pub fn nonce() -> Result<[u8; NONCE_BYTES], KeyError> {
    let mut nonce = [0_u8; NONCE_BYTES];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_unspecified| KeyError::Random)?;
    Ok(nonce)
}

/// What a proof covers: both nonces and both pins of one TLS session.
#[derive(Debug, Clone, Copy)]
pub struct Session<'a> {
    /// The master's nonce.
    pub master_nonce: &'a [u8],
    /// The worker's nonce.
    pub worker_nonce: &'a [u8],
    /// The pin of the master's key in this session.
    pub master_pin: &'a KeyPin,
    /// The pin of the worker's key in this session.
    pub worker_pin: &'a KeyPin,
}

fn message(label: &[u8], session: &Session<'_>) -> Vec<u8> {
    // Every piece but the label has a fixed length, so the concatenation is
    // unambiguous; the label ends with a zero byte.
    let mut message = Vec::with_capacity(label.len() + 1 + 2 * NONCE_BYTES + 128);
    message.extend_from_slice(label);
    message.push(0);
    message.extend_from_slice(session.master_nonce);
    message.extend_from_slice(session.worker_nonce);
    message.extend_from_slice(session.master_pin.as_str().as_bytes());
    message.extend_from_slice(session.worker_pin.as_str().as_bytes());
    message
}

/// The worker's proof that it knows the code.
pub fn worker_proof(code: &Code, session: &Session<'_>) -> Vec<u8> {
    hmac::sign(&code.key(), &message(WORKER_LABEL, session))
        .as_ref()
        .to_vec()
}

/// The master's proof that it knows the code.
pub fn master_proof(code: &Code, session: &Session<'_>) -> Vec<u8> {
    hmac::sign(&code.key(), &message(MASTER_LABEL, session))
        .as_ref()
        .to_vec()
}

/// Whether the worker's proof matches the code and the session, compared in
/// constant time.
pub fn verify_worker_proof(code: &Code, session: &Session<'_>, proof: &[u8]) -> bool {
    hmac::verify(&code.key(), &message(WORKER_LABEL, session), proof).is_ok()
}

/// Whether the master's proof matches the code and the session, compared in
/// constant time.
pub fn verify_master_proof(code: &Code, session: &Session<'_>, proof: &[u8]) -> bool {
    hmac::verify(&code.key(), &message(MASTER_LABEL, session), proof).is_ok()
}

#[cfg(test)]
mod tests {
    use super::{
        CODE_CHARS, Code, Session, master_proof, nonce, verify_master_proof, verify_worker_proof,
        worker_proof,
    };
    use crate::keys::LinkKey;

    #[test]
    fn a_code_has_100_bits_in_groups_and_reads_back_as_the_owner_types_it() {
        let code = Code::generate().unwrap();

        let shown = code.display();

        assert_eq!(code.canonical().len(), CODE_CHARS);
        assert_eq!(shown.len(), CODE_CHARS + 4);
        assert_eq!(shown.split('-').count(), 5);
        assert_eq!(Code::parse(&shown), Some(code.clone()));
        assert_eq!(
            Code::parse(&shown.to_ascii_lowercase().replace('-', " ")),
            Some(code)
        );
        assert_ne!(Code::generate().unwrap(), Code::generate().unwrap());
        assert_eq!(
            Code::parse("ilo0-0000-0000-0000-0000").map(|code| code.canonical().to_owned()),
            Some("11000000000000000000".to_owned())
        );
        assert_eq!(
            Code::parse("UUUU-0000-0000-0000-0000"),
            None,
            "U is not in the alphabet"
        );
        assert_eq!(Code::parse("0000-0000"), None);
        assert_eq!(
            format!("{:?}", Code::parse("0000-0000-0000-0000-0001").unwrap()),
            "Code(..)"
        );
    }

    #[test]
    fn a_proof_holds_only_for_its_code_its_side_and_its_session() {
        let code = Code::generate().unwrap();
        let other = Code::generate().unwrap();
        let (master, worker, relay) = (
            LinkKey::generate().unwrap().pin(),
            LinkKey::generate().unwrap().pin(),
            LinkKey::generate().unwrap().pin(),
        );
        let (master_nonce, worker_nonce) = (nonce().unwrap(), nonce().unwrap());
        let session = Session {
            master_nonce: &master_nonce,
            worker_nonce: &worker_nonce,
            master_pin: &master,
            worker_pin: &worker,
        };
        let relayed = Session {
            worker_pin: &relay,
            ..session
        };

        let from_worker = worker_proof(&code, &session);
        let from_master = master_proof(&code, &session);

        assert!(verify_worker_proof(&code, &session, &from_worker));
        assert!(verify_master_proof(&code, &session, &from_master));
        assert!(
            !verify_worker_proof(&other, &session, &from_worker),
            "another code"
        );
        assert!(
            !verify_master_proof(&code, &session, &from_worker),
            "the other side's proof"
        );
        assert!(
            !verify_worker_proof(&code, &relayed, &from_worker),
            "a key in the middle"
        );
    }
}
