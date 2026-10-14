//! The redacting secret type (security architecture, rules for handling
//! secrets).

use std::fmt;

use serde::{Deserialize, Deserializer};
use zeroize::Zeroizing;

/// A secret value, such as a provider credential.
///
/// `Secret` clears its memory when it is dropped and never shows its value when
/// formatted: `Debug` prints a fixed placeholder, and there is no `Display`, no
/// `Serialize` and no conversion back into a `String`, so a secret cannot reach
/// a log, the database or an IPC response by accident. Code that has to hand
/// the value to its destination, such as the credential store or a request
/// header, calls [`Secret::expose`].
///
/// A secret can arrive in a command's arguments, so it implements
/// `Deserialize`: the owner types a credential into the webview, which sends it
/// to the core once. It never travels back.
#[derive(Clone)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    /// Wraps a value. The `String` moves into the secret, so no other copy of
    /// it remains in the caller.
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    /// Returns the value.
    ///
    /// Only the code that delivers the secret to its destination calls this.
    /// The returned `&str` must not be logged, stored, displayed or copied into
    /// a type that outlives the call.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether the value is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

impl specta::Type for Secret {
    fn definition(types: &mut specta::Types) -> specta::datatype::DataType {
        <String as specta::Type>::definition(types)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::Secret;

    const PLANTED: &str = "sk-planted-test-secret-0123456789";

    #[test]
    fn debug_formatting_never_shows_the_value() {
        let secret = Secret::new(PLANTED.to_owned());

        let debug = format!("{secret:?}");
        let pretty = format!("{secret:#?}");
        let nested = format!("{:?}", Some(vec![secret]));

        for output in [debug, pretty, nested] {
            assert!(!output.contains(PLANTED), "{output}");
            assert!(output.contains("<redacted>"), "{output}");
        }
    }

    #[test]
    fn deserializes_from_a_string() {
        let secret: Secret = serde_json::from_str(&format!("\"{PLANTED}\"")).unwrap();

        assert_eq!(secret.expose(), PLANTED);
    }

    #[test]
    fn expose_returns_the_value() {
        let secret = Secret::from(PLANTED.to_owned());

        assert_eq!(secret.expose(), PLANTED);
        assert!(!secret.is_empty());
        assert!(Secret::new(String::new()).is_empty());
    }
}
