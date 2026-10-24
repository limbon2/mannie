//! The servers' secrets in the credential store (ADR-0043, decision 6): the
//! values of a server's secret variables or headers, and its OAuth state,
//! each as one text of the keyed families `variable`, `header` and `oauth`.
//!
//! The Windows credential store keeps at most 1,280 UTF-16 units in one
//! entry (docs/roadmap/m12-validation.md, section 5), so a text is split into
//! numbered parts, `<family>/<server ID>-<part>`, and the server's row counts
//! them. A save writes the new parts first and deletes the parts past them
//! after the row counts the new ones, so a server never counts a part that
//! is not stored. A text that does not read back whole, as after a save that
//! a crash cut short, reads as missing, and the owner enters it again.

use std::sync::Arc;

use mannie_plugin_sdk::contracts::Secret;
use mannie_plugin_sdk::services::secrets::{SecretsApi, SecretsError};

/// The most UTF-16 units of one part, below the credential store's limit.
const PART_UNITS: usize = 1_000;

/// The most parts of one text, which the row's CHECK allows.
pub(crate) const MAX_PARTS: u32 = 1_000;

/// A family of the plugin's entries in the credential store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    /// The secret variables of a local server.
    Variable,
    /// The secret headers of a remote server.
    Header,
    /// The OAuth state of a remote server: its client secret, its
    /// registration and its tokens.
    OAuth,
}

impl Family {
    fn name(self) -> &'static str {
        match self {
            Self::Variable => "variable",
            Self::Header => "header",
            Self::OAuth => "oauth",
        }
    }
}

/// The entry of one part of a text.
fn entry(family: Family, server_id: &str, part: u32) -> String {
    format!("{}/{server_id}-{part}", family.name())
}

/// A text in parts of at most [`PART_UNITS`] UTF-16 units, cut between
/// characters.
fn split(text: &str) -> Vec<Secret> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut units = 0;
    for character in text.chars() {
        let width = character.len_utf16();
        if units + width > PART_UNITS {
            parts.push(Secret::new(std::mem::take(&mut current)));
            units = 0;
        }
        current.push(character);
        units += width;
    }
    if !current.is_empty() {
        parts.push(Secret::new(current));
    }
    parts
}

/// How many parts [`Vault::save`] writes of a text.
pub(crate) fn part_count(text: &Secret) -> u32 {
    u32::try_from(split(text.expose()).len()).unwrap_or(u32::MAX)
}

/// The servers' secrets, through the plugin's entries of the credential
/// store.
#[derive(Debug, Clone)]
pub(crate) struct Vault {
    secrets: Arc<dyn SecretsApi>,
}

impl Vault {
    pub(crate) fn new(secrets: Arc<dyn SecretsApi>) -> Self {
        Self { secrets }
    }

    /// Writes a text in parts and returns how many. An empty text has none.
    /// The parts past them that an earlier text left are the caller's to
    /// delete with [`Self::delete_from`] once the row counts the new ones.
    pub(crate) async fn save(
        &self,
        family: Family,
        server_id: &str,
        text: &Secret,
    ) -> Result<u32, SecretsError> {
        let parts = split(text.expose());
        let count = u32::try_from(parts.len()).unwrap_or(u32::MAX);
        if count > MAX_PARTS {
            return Err(SecretsError::Unavailable);
        }
        for (index, part) in (0..count).zip(parts) {
            self.secrets
                .save(&entry(family, server_id, index), part)
                .await?;
        }
        Ok(count)
    }

    /// Reads a text of `parts` parts, or `None` when a part is missing.
    pub(crate) async fn load(
        &self,
        family: Family,
        server_id: &str,
        parts: u32,
    ) -> Result<Option<Secret>, SecretsError> {
        let mut text = Secret::new(String::new());
        for index in 0..parts.min(MAX_PARTS) {
            let Some(part) = self.secrets.load(&entry(family, server_id, index)).await? else {
                return Ok(None);
            };
            let mut joined = String::with_capacity(text.expose().len() + part.expose().len());
            joined.push_str(text.expose());
            joined.push_str(part.expose());
            text = Secret::new(joined);
        }
        Ok(Some(text))
    }

    /// Deletes the parts from `first` up to `count`, the parts that a row
    /// counted before.
    pub(crate) async fn delete_from(
        &self,
        family: Family,
        server_id: &str,
        first: u32,
        count: u32,
    ) -> Result<(), SecretsError> {
        for index in first..count.min(MAX_PARTS) {
            self.secrets
                .delete(&entry(family, server_id, index))
                .await?;
        }
        Ok(())
    }
}

/// The secret values of a server's variables or headers, by name, as the
/// text that the credential store keeps: a JSON array of pairs.
pub(crate) fn encode_values(values: &[(String, Secret)]) -> Secret {
    let pairs: Vec<(&str, &str)> = values
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose()))
        .collect();
    // A list of string pairs always serializes.
    Secret::new(serde_json::to_string(&pairs).unwrap_or_default())
}

/// The values that [`encode_values`] wrote, or `None` for a text that is
/// not one.
pub(crate) fn decode_values(text: &Secret) -> Option<Vec<(String, Secret)>> {
    if text.is_empty() {
        return Some(Vec::new());
    }
    serde_json::from_str(text.expose()).ok()
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::Secret;

    use super::{Family, PART_UNITS, decode_values, encode_values, entry, split};

    #[test]
    fn a_long_text_splits_between_characters_within_the_stores_limit() {
        let text = format!("{}é{}", "a".repeat(PART_UNITS - 1), "😀".repeat(600));

        let parts = split(&text);

        let joined: String = parts.iter().map(Secret::expose).collect();
        assert_eq!(joined, text);
        for part in &parts {
            assert!(part.expose().encode_utf16().count() <= PART_UNITS);
        }
        assert_eq!(parts.len(), 3);
        assert!(split("").is_empty());
    }

    #[test]
    fn values_cross_the_store_as_pairs_and_entries_name_their_part() {
        let values = vec![
            (
                "GITHUB_TOKEN".to_owned(),
                Secret::new("ghp_planted".to_owned()),
            ),
            ("OTHER".to_owned(), Secret::new("\"quoted\"".to_owned())),
        ];

        let decoded = decode_values(&encode_values(&values)).unwrap();

        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[1].1.expose(), "\"quoted\"");
        assert!(decode_values(&Secret::new("not json".to_owned())).is_none());
        assert_eq!(
            entry(Family::Header, "0199e0f2-0000-7000-8000-0000000000aa", 2),
            "header/0199e0f2-0000-7000-8000-0000000000aa-2"
        );
    }
}
