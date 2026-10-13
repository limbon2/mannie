//! The interface settings section: the language of the desktop UI
//! (ADR-0019), and the IPC types that read and change it.
//!
//! The core stores only the owner's choice. The UI resolves the locale it
//! shows: the chosen locale when a message catalog exists for it, otherwise the
//! operating system's language when a catalog exists for it, otherwise the
//! base locale, English.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use specta::Type;

use crate::settings::SettingsSection;

/// The longest locale tag Mannie accepts. Real tags are far shorter; the
/// limit keeps arbitrary input out of the settings.
const MAX_LOCALE_TAG_LENGTH: usize = 35;

/// A well-formed BCP 47 language tag, such as `en`, `pt-BR` or `zh-Hant-TW`.
///
/// Parsing checks the syntax and puts every subtag in its conventional case,
/// so equal tags compare equal. It does not check that the language exists:
/// the UI decides which tags have a message catalog.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Type)]
pub struct LocaleTag(String);

impl LocaleTag {
    /// Parses a language tag.
    pub fn parse(tag: &str) -> Result<Self, InvalidLocaleTag> {
        let invalid = || InvalidLocaleTag {
            tag: tag.to_owned(),
        };
        let is_empty = tag.is_empty();
        let is_too_long = tag.len() > MAX_LOCALE_TAG_LENGTH;
        if is_empty || is_too_long {
            return Err(invalid());
        }

        let mut canonical = String::with_capacity(tag.len());
        // After a singleton such as `x` or `u`, subtags are extensions or
        // private use and keep lowercase (RFC 5646, section 2.1.1).
        let mut after_singleton = false;
        for (index, subtag) in tag.split('-').enumerate() {
            let is_alphanumeric = subtag.bytes().all(|byte| byte.is_ascii_alphanumeric());
            let has_valid_length = (1..=8).contains(&subtag.len());
            if !(is_alphanumeric && has_valid_length) {
                return Err(invalid());
            }
            if index == 0 {
                let is_letters = subtag.bytes().all(|byte| byte.is_ascii_alphabetic());
                let is_language_length = (2..=8).contains(&subtag.len());
                if !(is_letters && is_language_length) {
                    return Err(invalid());
                }
                canonical.push_str(&subtag.to_ascii_lowercase());
                continue;
            }
            canonical.push('-');
            if subtag.len() == 1 {
                after_singleton = true;
            }
            if after_singleton {
                canonical.push_str(&subtag.to_ascii_lowercase());
            } else {
                canonical.push_str(&conventional_case(subtag));
            }
        }
        Ok(Self(canonical))
    }

    /// The tag in its conventional case.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Scripts are title case (`Hant`), regions upper case (`BR`), and everything
/// else lower case (RFC 5646, section 2.1.1).
fn conventional_case(subtag: &str) -> String {
    let is_letters = subtag.bytes().all(|byte| byte.is_ascii_alphabetic());
    match (subtag.len(), is_letters) {
        (4, true) => {
            let lower = subtag.to_ascii_lowercase();
            let (first, rest) = lower.split_at(1);
            format!("{}{rest}", first.to_ascii_uppercase())
        }
        (2, true) => subtag.to_ascii_uppercase(),
        _ => subtag.to_ascii_lowercase(),
    }
}

impl fmt::Display for LocaleTag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for LocaleTag {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for LocaleTag {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let tag = String::deserialize(deserializer)?;
        Self::parse(&tag).map_err(serde::de::Error::custom)
    }
}

/// A string that is not a well-formed language tag.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{tag:?} is not a well-formed language tag")]
pub struct InvalidLocaleTag {
    /// The rejected input.
    pub tag: String,
}

/// The interface settings section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceSettings {
    /// The locale the owner chose, or `None` to follow the operating system.
    pub locale: Option<LocaleTag>,
}

impl SettingsSection for InterfaceSettings {
    const KEY: &'static str = "interface";
    const VERSION: u32 = 1;
}

/// The argument of the `update_interface_settings` command: the complete new
/// section. The locale arrives as a plain string so that the core, not the IPC
/// layer, rejects a malformed tag with a typed error.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInterfaceSettings {
    /// The locale to use, or `None` to follow the operating system.
    pub locale: Option<String>,
}

impl UpdateInterfaceSettings {
    /// Checks the request and turns it into the section to store.
    pub fn into_settings(self) -> Result<InterfaceSettings, InterfaceSettingsError> {
        let locale =
            match self.locale {
                Some(tag) => Some(LocaleTag::parse(&tag).map_err(|invalid| {
                    InterfaceSettingsError::InvalidLocale { tag: invalid.tag }
                })?),
                None => None,
            };
        Ok(InterfaceSettings { locale })
    }
}

/// Why reading or changing the interface settings failed. The UI shows the
/// message for the code from its catalog (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum InterfaceSettingsError {
    /// The requested locale is not a well-formed language tag.
    #[error("{tag:?} is not a well-formed language tag")]
    InvalidLocale {
        /// The rejected tag.
        tag: String,
    },
    /// The stored settings were written by a newer version of Mannie, which
    /// this version does not overwrite.
    #[error("the interface settings were written by a newer version of Mannie")]
    WrittenByNewerVersion,
    /// The store could not read or save the settings. The log has the cause.
    #[error("the interface settings could not be read or saved")]
    StorageFailed,
    /// The core started without its database, so there are no settings to
    /// read or save. The startup status says why.
    #[error("the database is not open")]
    StoreUnavailable,
}

impl InterfaceSettingsError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidLocale { .. } => "interface.invalid_locale",
            Self::WrittenByNewerVersion => "interface.written_by_newer_version",
            Self::StorageFailed => "interface.storage_failed",
            Self::StoreUnavailable => "interface.store_unavailable",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{InterfaceSettings, InterfaceSettingsError, LocaleTag, UpdateInterfaceSettings};

    #[test]
    fn parse_puts_subtags_in_conventional_case() {
        let cases = [
            ("en", "en"),
            ("EN", "en"),
            ("pt-br", "pt-BR"),
            ("zh-hant-tw", "zh-Hant-TW"),
            ("es-419", "es-419"),
            ("en-XA", "en-XA"),
            ("de-CH-1996", "de-CH-1996"),
            ("en-x-ab-pseudo", "en-x-ab-pseudo"),
            ("sr-Latn-u-ca-gregory", "sr-Latn-u-ca-gregory"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                LocaleTag::parse(input).map(|tag| tag.to_string()),
                Ok(expected.to_owned()),
                "{input}"
            );
        }
    }

    #[test]
    fn parse_rejects_malformed_tags() {
        let long = format!("en-{}", "a".repeat(40));
        let cases = [
            "",
            "e",
            "123",
            "en_US",
            "en-",
            "-en",
            "en--US",
            "en-toolongsubtag",
            "en US",
            "ру",
            &long,
        ];
        for input in cases {
            assert!(
                LocaleTag::parse(input).is_err(),
                "{input:?} should be rejected"
            );
        }
    }

    #[test]
    fn deserializing_checks_the_tag() {
        let valid: InterfaceSettings = serde_json::from_str(r#"{"locale":"pt-br"}"#).unwrap();
        let invalid = serde_json::from_str::<InterfaceSettings>(r#"{"locale":"not a tag"}"#);

        assert_eq!(
            valid.locale.map(|tag| tag.to_string()),
            Some("pt-BR".to_owned())
        );
        invalid.unwrap_err();
    }

    #[test]
    fn serializes_as_a_versionable_document() {
        let settings = InterfaceSettings {
            locale: Some(LocaleTag::parse("en").unwrap()),
        };

        assert_eq!(
            serde_json::to_string(&settings).unwrap(),
            r#"{"locale":"en"}"#
        );
        assert_eq!(
            serde_json::to_string(&InterfaceSettings::default()).unwrap(),
            r#"{"locale":null}"#
        );
    }

    #[test]
    fn update_rejects_a_malformed_locale_with_a_typed_error() {
        let update = UpdateInterfaceSettings {
            locale: Some("en_US".to_owned()),
        };

        let error = update.into_settings().unwrap_err();

        assert_eq!(
            error,
            InterfaceSettingsError::InvalidLocale {
                tag: "en_US".to_owned()
            }
        );
        assert_eq!(error.code(), "interface.invalid_locale");
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            serde_json::json!({ "code": "invalid_locale", "tag": "en_US" })
        );
    }

    #[test]
    fn update_accepts_a_locale_or_none() {
        let chosen = UpdateInterfaceSettings {
            locale: Some("fr-ca".to_owned()),
        }
        .into_settings()
        .unwrap();
        let system = UpdateInterfaceSettings { locale: None }
            .into_settings()
            .unwrap();

        assert_eq!(
            chosen.locale.map(|tag| tag.to_string()),
            Some("fr-CA".to_owned())
        );
        assert_eq!(system, InterfaceSettings::default());
    }
}
