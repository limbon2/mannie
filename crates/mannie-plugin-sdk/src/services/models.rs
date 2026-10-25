//! The service `models` (ADR-0047): the embedding and rerank models of the
//! owner's search profiles. A plugin names a profile by its ID and never sees
//! its server, its credential or its prefixes. The core puts the profile's
//! prefix before each text that it embeds, calls the server, and retries a
//! transient failure as it retries a reply.
//!
//! Every call sends the plugin's texts to the server that the owner chose,
//! so the service is sensitive.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::service::Service;
use crate::{BoxFuture, BoxStream};

/// The most texts that one call takes.
pub const MAX_INPUTS: usize = 64;

/// The most bytes of one text, or of a query, that a call takes.
pub const MAX_INPUT_BYTES: usize = 32 * 1024;

/// The service `models`, major 1.
#[derive(Debug)]
pub enum Models {}

impl Service for Models {
    const NAME: &'static str = "models";
    const MAJOR: u16 = 1;
    const SENSITIVE: bool = true;
    type Api = dyn ModelsApi;
}

/// The embedding and rerank models of the owner's search profiles.
pub trait ModelsApi: Send + Sync + fmt::Debug {
    /// The search profiles of a kind, the oldest first.
    fn search_profiles(
        &self,
        kind: SearchModelKind,
    ) -> BoxFuture<'static, Result<Vec<SearchModel>, ModelsError>>;

    /// Embeds texts with an embedding profile, as queries or as documents,
    /// and returns one vector for each, in their order, all of one
    /// dimension. A call takes 1 to [`MAX_INPUTS`] texts of at most
    /// [`MAX_INPUT_BYTES`] each.
    fn embed(
        &self,
        profile: &str,
        kind: InputKind,
        inputs: Vec<String>,
    ) -> BoxFuture<'static, Result<Vec<Vec<f32>>, ModelsError>>;

    /// Scores passages against a query with a rerank profile, and returns one
    /// score for each, in their order, as a probability from 0 to 1. A call
    /// takes 1 to [`MAX_INPUTS`] passages of at most [`MAX_INPUT_BYTES`]
    /// each.
    fn rerank(
        &self,
        profile: &str,
        query: String,
        documents: Vec<String>,
    ) -> BoxFuture<'static, Result<Vec<f32>, ModelsError>>;

    /// The IDs of the search profiles that change or are deleted, from now
    /// on. A plugin reads the profile again, or finds it gone.
    fn changes(&self) -> BoxStream<'static, String>;
}

/// What the model of a search profile does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SearchModelKind {
    /// It turns texts into vectors.
    Embeddings,
    /// It scores passages against a query.
    Rerank,
}

/// A search profile as a plugin knows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct SearchModel {
    /// The profile's ID, which the plugin names in its calls.
    pub id: String,
    /// The profile's name, as the owner gave it.
    pub name: String,
    /// The model's name on its server.
    pub model: String,
    /// What the model does.
    pub kind: SearchModelKind,
    /// What a document's vector means: it changes when the profile's model
    /// or its document prefix changes, and only then, so a plugin that keeps
    /// vectors embeds its documents again.
    pub vector_key: String,
}

impl SearchModel {
    /// A profile with this ID, name, model, kind and key of its vectors.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        model: impl Into<String>,
        kind: SearchModelKind,
        vector_key: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            model: model.into(),
            kind,
            vector_key: vector_key.into(),
        }
    }
}

/// What the texts of an embedding are, which decides the profile's prefix
/// that goes before each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InputKind {
    /// What is searched for, such as the owner's message.
    Query,
    /// What is searched, such as a passage of a document.
    Document,
}

/// Why a call of a search profile failed. The core adds reasons by
/// addition, so a plugin treats a reason that it does not know as a failure
/// of the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum ModelsError {
    /// No search profile has the ID.
    #[error("no search profile has the ID")]
    ProfileNotFound,
    /// The profile is of the other kind, such as a rerank profile for an
    /// embedding.
    #[error("the search profile is of another kind")]
    WrongKind,
    /// A call without texts, with more than [`MAX_INPUTS`], or with one longer
    /// than [`MAX_INPUT_BYTES`].
    #[error("the texts of the call are not valid")]
    InvalidInputs,
    /// The server could not be reached, timed out or failed, also after the
    /// core tried again.
    #[error("the server of the search profile could not be reached")]
    Unreachable,
    /// The server refused the call, with the core's failure code of a model
    /// call, such as `invalid_credential` or `model_not_found`.
    #[error("the server of the search profile refused the call: {failure}")]
    Refused {
        /// The failure code.
        failure: String,
    },
    /// A text is longer than the model takes.
    #[error("a text is longer than the model takes")]
    InputTooLong,
    /// The server answered with something other than the API's shape.
    #[error("the server of the search profile answered with something unexpected")]
    InvalidResponse,
    /// The profile's credential could not be read from the credential store.
    #[error("the credential of the search profile could not be read")]
    CredentialUnavailable,
    /// The core started without its database.
    #[error("the search profiles cannot be read")]
    Unavailable,
}

impl ModelsError {
    /// The error's stable code, which a plugin logs and maps to its own.
    pub fn code(&self) -> &'static str {
        match self {
            Self::ProfileNotFound => "models.profile_not_found",
            Self::WrongKind => "models.wrong_kind",
            Self::InvalidInputs => "models.invalid_inputs",
            Self::Unreachable => "models.unreachable",
            Self::Refused { .. } => "models.refused",
            Self::InputTooLong => "models.input_too_long",
            Self::InvalidResponse => "models.invalid_response",
            Self::CredentialUnavailable => "models.credential_unavailable",
            Self::Unavailable => "models.unavailable",
        }
    }
}

/// Whether a call's texts fit the service's limits: 1 to [`MAX_INPUTS`],
/// each of at most [`MAX_INPUT_BYTES`]. The core checks them, and a plugin
/// can check before it calls.
pub fn inputs_fit(inputs: &[String]) -> bool {
    let count_fits = !inputs.is_empty() && inputs.len() <= MAX_INPUTS;
    count_fits && inputs.iter().all(|input| input.len() <= MAX_INPUT_BYTES)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{MAX_INPUT_BYTES, MAX_INPUTS, ModelsError, inputs_fit};

    #[test]
    fn inputs_fit_between_one_and_the_most_with_each_within_its_bytes() {
        let one = vec!["text".to_owned()];
        let most = vec!["text".to_owned(); MAX_INPUTS];
        let too_many = vec!["text".to_owned(); MAX_INPUTS + 1];
        let too_long = vec!["x".repeat(MAX_INPUT_BYTES + 1)];

        assert!(inputs_fit(&one));
        assert!(inputs_fit(&most));
        assert!(!inputs_fit(&too_many));
        assert!(!inputs_fit(&too_long));
        assert!(!inputs_fit(&[]));
    }

    #[test]
    fn every_error_has_a_code_of_its_own_and_serializes_with_it() {
        let errors = [
            ModelsError::ProfileNotFound,
            ModelsError::WrongKind,
            ModelsError::InvalidInputs,
            ModelsError::Unreachable,
            ModelsError::Refused {
                failure: "invalid_credential".to_owned(),
            },
            ModelsError::InputTooLong,
            ModelsError::InvalidResponse,
            ModelsError::CredentialUnavailable,
            ModelsError::Unavailable,
        ];

        let codes: BTreeSet<&str> = errors.iter().map(ModelsError::code).collect();

        assert_eq!(codes.len(), errors.len());
        assert_eq!(
            serde_json::to_value(&errors[4]).unwrap(),
            serde_json::json!({ "code": "refused", "failure": "invalid_credential" })
        );
    }
}
