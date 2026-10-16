//! The link's long-term keys (docs/features/workers/pairing-and-link.md, keys
//! and pins): an Ed25519 key pair for each side, its `SubjectPublicKeyInfo`,
//! which TLS presents as a raw public key (RFC 7250), and its pin, the
//! SHA-256 of that `SubjectPublicKeyInfo`.

use std::fmt;
use std::fmt::Write as _;
use std::sync::Arc;

use ring::digest::{SHA256, digest};
use ring::rand::SystemRandom;
use ring::signature::Ed25519KeyPair;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use rustls::sign::{CertifiedKey, SigningKey};
use zeroize::Zeroizing;

/// The hexadecimal digits of a pin.
const PIN_CHARS: usize = 64;

/// The characters of a pin that the owner compares on screen.
const SHORT_PIN_CHARS: usize = 12;

/// Why a key could not be made or read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// The system's random generator failed.
    #[error("the system's random generator failed")]
    Random,
    /// The stored key is not an Ed25519 key in PKCS#8.
    #[error("the stored key is not valid")]
    Invalid,
}

/// The pin of a public key: the SHA-256 of its `SubjectPublicKeyInfo`, as 64
/// lowercase hexadecimal characters. Pins are not secret.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeyPin(String);

impl KeyPin {
    /// The pin of a `SubjectPublicKeyInfo`.
    pub fn of_spki(spki: &[u8]) -> Self {
        let hash = digest(&SHA256, spki);
        let mut text = String::with_capacity(PIN_CHARS);
        for byte in hash.as_ref() {
            // Writing to a String cannot fail.
            let _written = write!(text, "{byte:02x}");
        }
        Self(text)
    }

    /// A pin as it was written, or `None` when it is not 64 hexadecimal
    /// characters.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let valid = text.len() == PIN_CHARS && text.chars().all(|c| c.is_ascii_hexdigit());
        valid.then(|| Self(text.to_ascii_lowercase()))
    }

    /// The pin as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The first characters, which the owner compares on screen and the logs
    /// write.
    pub fn short(&self) -> &str {
        &self.0[..SHORT_PIN_CHARS]
    }
}

impl fmt::Debug for KeyPin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "KeyPin({})", self.short())
    }
}

impl fmt::Display for KeyPin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A side's long-term key pair. The private key is cleared from memory when
/// the value is dropped.
pub struct LinkKey {
    pkcs8: Zeroizing<Vec<u8>>,
    spki: Vec<u8>,
    signing: Arc<dyn SigningKey>,
}

impl fmt::Debug for LinkKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LinkKey")
            .field("pin", &self.pin())
            .finish_non_exhaustive()
    }
}

impl LinkKey {
    /// A new key pair.
    pub fn generate() -> Result<Self, KeyError> {
        let random = SystemRandom::new();
        let document =
            Ed25519KeyPair::generate_pkcs8(&random).map_err(|_unspecified| KeyError::Random)?;
        Self::from_pkcs8(document.as_ref().to_vec())
    }

    /// A key pair as [`LinkKey::pkcs8`] wrote it.
    pub fn from_pkcs8(pkcs8: Vec<u8>) -> Result<Self, KeyError> {
        let pkcs8 = Zeroizing::new(pkcs8);
        // ring checks that the private key and the public key in the
        // document belong together.
        Ed25519KeyPair::from_pkcs8(&pkcs8).map_err(|_rejected| KeyError::Invalid)?;
        let der = PrivatePkcs8KeyDer::from(pkcs8.as_slice());
        let signing =
            rustls::crypto::ring::sign::any_eddsa_type(&der).map_err(|_error| KeyError::Invalid)?;
        let spki = signing
            .public_key()
            .ok_or(KeyError::Invalid)?
            .as_ref()
            .to_vec();
        Ok(Self {
            pkcs8,
            spki,
            signing,
        })
    }

    /// The key pair as a PKCS#8 document, to store. It holds the private key.
    pub fn pkcs8(&self) -> &[u8] {
        &self.pkcs8
    }

    /// The public key's `SubjectPublicKeyInfo`.
    pub fn spki(&self) -> &[u8] {
        &self.spki
    }

    /// The public key's pin.
    pub fn pin(&self) -> KeyPin {
        KeyPin::of_spki(&self.spki)
    }

    /// The key as TLS presents it: the public key alone, as a raw public
    /// key, with the signing key.
    pub(crate) fn certified_key(&self) -> Arc<CertifiedKey> {
        Arc::new(CertifiedKey::new(
            vec![CertificateDer::from(self.spki.clone())],
            Arc::clone(&self.signing),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{KeyError, KeyPin, LinkKey};

    #[test]
    fn a_key_reads_back_with_the_same_pin_and_pins_are_hexadecimal() {
        let key = LinkKey::generate().unwrap();

        let again = LinkKey::from_pkcs8(key.pkcs8().to_vec()).unwrap();

        assert_eq!(again.pin(), key.pin());
        assert_eq!(key.pin().as_str().len(), 64);
        assert_eq!(key.pin().short().len(), 12);
        assert_eq!(
            KeyPin::parse(&key.pin().as_str().to_ascii_uppercase()),
            Some(key.pin())
        );
        assert_eq!(KeyPin::parse("abc"), None);
        assert_eq!(KeyPin::parse(&"g".repeat(64)), None);
        assert_ne!(LinkKey::generate().unwrap().pin(), key.pin());
    }

    #[test]
    fn a_damaged_key_is_refused() {
        let key = LinkKey::generate().unwrap();
        let mut damaged = key.pkcs8().to_vec();
        let last = damaged.len() - 1;
        damaged[last] ^= 0xff;

        assert!(matches!(
            LinkKey::from_pkcs8(damaged),
            Err(KeyError::Invalid)
        ));
        assert!(matches!(
            LinkKey::from_pkcs8(vec![1, 2, 3]),
            Err(KeyError::Invalid)
        ));
    }

    #[test]
    fn the_debug_form_shows_the_pin_and_never_the_key() {
        let key = LinkKey::generate().unwrap();

        let shown = format!("{key:?}");

        assert!(shown.contains(key.pin().short()), "{shown}");
        assert!(!shown.contains("pkcs8"), "{shown}");
    }
}
