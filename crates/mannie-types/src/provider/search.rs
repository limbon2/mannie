//! Search profiles (ADR-0047): one embedding or rerank model on one server,
//! with an optional credential in the operating system's credential store.
//! Nothing of a server or a model is built in: the owner names the server,
//! the model and the prefixes of every profile.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::clock::UtcMillis;
use crate::ids::ProfileId;
use crate::secret::Secret;

use super::profile::{
    CredentialChange, MAX_NAME_CHARACTERS, ModelName, ProviderProfileError, ServerKind,
};
use super::test_report::CheckOutcome;

/// The longest prefix of an embedding profile, in characters.
pub const MAX_PREFIX_CHARACTERS: usize = 1_000;

/// What the model of a search profile does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    /// It turns texts into vectors, through the server's `/embeddings`.
    Embeddings,
    /// It scores passages against a query, through the server's `/rerank`.
    Rerank,
}

impl SearchKind {
    /// Every kind.
    pub const ALL: [Self; 2] = [Self::Embeddings, Self::Rerank];

    /// The kind's name as the store keeps it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Embeddings => "embeddings",
            Self::Rerank => "rerank",
        }
    }

    /// The kind with this name.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

/// A search profile as the UI shows it. The credential never leaves the core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchProfile {
    /// The profile.
    pub id: ProfileId,
    /// Its name.
    pub name: String,
    /// What its model does.
    pub kind: SearchKind,
    /// The server's base URL.
    pub base_url: String,
    /// The model.
    pub model: String,
    /// Whether a credential is stored for it.
    pub has_credential: bool,
    /// The text before each query that an embedding profile embeds, or empty.
    pub query_prefix: String,
    /// The text before each document that an embedding profile embeds, or
    /// empty.
    pub document_prefix: String,
    /// The server the test recognized, if it ran.
    pub server: Option<ServerKind>,
    /// When it was created.
    pub created_at: UtcMillis,
    /// When it last changed.
    pub updated_at: UtcMillis,
}

/// What the owner entered for a search profile, before the core checks it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchProfileDraft {
    /// The name.
    pub name: String,
    /// What the model does.
    pub kind: SearchKind,
    /// The base URL.
    pub base_url: String,
    /// The model.
    pub model: String,
    /// The text before each query, for an embedding profile; empty otherwise.
    pub query_prefix: String,
    /// The text before each document, for an embedding profile; empty
    /// otherwise.
    pub document_prefix: String,
}

/// The fields of a draft that need no URL parser, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedSearchFields {
    /// The name without surrounding white space.
    pub name: String,
    /// The model.
    pub model: ModelName,
    /// The query prefix, exactly as the owner wrote it.
    pub query_prefix: String,
    /// The document prefix, exactly as the owner wrote it.
    pub document_prefix: String,
}

impl SearchProfileDraft {
    /// Checks everything except the base URL, which the provider crate checks:
    /// the name, the model and the prefixes. A prefix keeps its white space,
    /// which belongs to the text that a model expects, and only an embedding
    /// profile may have one.
    pub fn check_fields(&self) -> Result<CheckedSearchFields, ProviderProfileError> {
        let name = self.name.trim();
        let name_is_valid = !name.is_empty()
            && name.chars().count() <= MAX_NAME_CHARACTERS
            && !name.chars().any(char::is_control);
        if !name_is_valid {
            return Err(ProviderProfileError::InvalidName);
        }
        let model = ModelName::parse(&self.model)?;
        let prefixes = [&self.query_prefix, &self.document_prefix];
        let has_prefix = prefixes.iter().any(|prefix| !prefix.is_empty());
        if self.kind == SearchKind::Rerank && has_prefix {
            return Err(invalid_prefix());
        }
        if !prefixes.into_iter().all(|prefix| is_valid_prefix(prefix)) {
            return Err(invalid_prefix());
        }
        Ok(CheckedSearchFields {
            name: name.to_owned(),
            model,
            query_prefix: self.query_prefix.clone(),
            document_prefix: self.document_prefix.clone(),
        })
    }
}

fn invalid_prefix() -> ProviderProfileError {
    ProviderProfileError::InvalidPrefix {
        max_characters: u32::try_from(MAX_PREFIX_CHARACTERS).unwrap_or(u32::MAX),
    }
}

/// A prefix fits its limit and holds no control characters but line breaks
/// and tabs, which instructions for a model may have.
fn is_valid_prefix(prefix: &str) -> bool {
    let fits = prefix.chars().count() <= MAX_PREFIX_CHARACTERS;
    let is_plain = prefix
        .chars()
        .all(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'));
    fits && is_plain
}

/// The argument of `create_search_profile`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateSearchProfile {
    /// The profile's fields.
    pub draft: SearchProfileDraft,
    /// The credential, if the server needs one.
    pub credential: Option<Secret>,
}

/// The argument of `update_search_profile`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSearchProfile {
    /// The profile.
    pub id: ProfileId,
    /// Its new fields.
    pub draft: SearchProfileDraft,
    /// What happens to its credential.
    pub credential: CredentialChange,
}

/// The checks of a search profile's test, in the order they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SearchCheckKind {
    /// The stored credential can be read.
    Credential,
    /// The server answers and accepts the credential.
    Server,
    /// The server knows the model.
    Model,
    /// A short text gets a vector.
    Embed,
    /// A passage that answers a question scores higher than one that does not.
    Rerank,
}

/// One check of a search profile's test and its outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchCheck {
    /// What was checked.
    pub check: SearchCheckKind,
    /// What happened.
    pub outcome: CheckOutcome,
}

/// The scores that the rerank check got, from 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RerankProbe {
    /// The score of the passage that answers the question.
    pub answering: f32,
    /// The score of the passage that does not.
    pub unrelated: f32,
}

/// What the test of a search profile found out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchTestReport {
    /// Each check, in the order it ran.
    pub checks: Vec<SearchCheck>,
    /// The server, when the test recognized it.
    pub server: Option<ServerKind>,
    /// The dimensions of the vectors, when the embed check got one.
    pub dimensions: Option<u32>,
    /// The scores, when the rerank check got them.
    pub scores: Option<RerankProbe>,
}

#[cfg(test)]
mod tests {
    use super::{MAX_PREFIX_CHARACTERS, SearchKind, SearchProfileDraft};
    use crate::provider::ProviderProfileError;

    fn draft(kind: SearchKind) -> SearchProfileDraft {
        SearchProfileDraft {
            name: "  GPU box embeddings ".to_owned(),
            kind,
            base_url: "http://gpu-box.local:8080/v1".to_owned(),
            model: " an-embedding-model ".to_owned(),
            query_prefix: String::new(),
            document_prefix: String::new(),
        }
    }

    #[test]
    fn kinds_round_trip_by_name() {
        for kind in SearchKind::ALL {
            assert_eq!(SearchKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(SearchKind::parse("chat"), None);
    }

    #[test]
    fn a_draft_is_trimmed_and_its_prefixes_kept_exactly() {
        let with_prefixes = SearchProfileDraft {
            query_prefix: "Instruct: find passages\nQuery: ".to_owned(),
            document_prefix: "passage: ".to_owned(),
            ..draft(SearchKind::Embeddings)
        };

        let checked = with_prefixes.check_fields().unwrap();

        assert_eq!(checked.name, "GPU box embeddings");
        assert_eq!(checked.model.as_str(), "an-embedding-model");
        assert_eq!(checked.query_prefix, "Instruct: find passages\nQuery: ");
        assert_eq!(checked.document_prefix, "passage: ");
    }

    #[test]
    fn prefixes_belong_to_embedding_profiles_and_keep_their_limits() {
        let reranker_with_prefix = SearchProfileDraft {
            query_prefix: "query: ".to_owned(),
            ..draft(SearchKind::Rerank)
        };
        let long = SearchProfileDraft {
            query_prefix: "q".repeat(MAX_PREFIX_CHARACTERS + 1),
            ..draft(SearchKind::Embeddings)
        };
        let control = SearchProfileDraft {
            document_prefix: "a\u{7}b".to_owned(),
            ..draft(SearchKind::Embeddings)
        };
        let refused = ProviderProfileError::InvalidPrefix {
            max_characters: 1_000,
        };

        assert_eq!(reranker_with_prefix.check_fields(), Err(refused.clone()));
        assert_eq!(long.check_fields(), Err(refused.clone()));
        assert_eq!(control.check_fields(), Err(refused));
        draft(SearchKind::Rerank).check_fields().unwrap();
    }

    #[test]
    fn names_and_models_follow_the_rules_of_chat_profiles() {
        let blank = SearchProfileDraft {
            name: " ".to_owned(),
            ..draft(SearchKind::Embeddings)
        };
        let no_model = SearchProfileDraft {
            model: String::new(),
            ..draft(SearchKind::Embeddings)
        };

        assert_eq!(blank.check_fields(), Err(ProviderProfileError::InvalidName));
        assert_eq!(
            no_model.check_fields(),
            Err(ProviderProfileError::InvalidModel)
        );
    }
}
