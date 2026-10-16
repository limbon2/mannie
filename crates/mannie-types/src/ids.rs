//! Identifiers of the domain's entities: UUIDv7 strings, created by the side
//! that creates the entity (docs/architecture/domain-model.md, identifiers and
//! time).
//!
//! Each kind of entity has its own type, so a run's ID cannot be passed where a
//! conversation's is expected. This crate only checks and carries IDs; the
//! runtime creates them from the injected clock.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The length of a UUID in its hyphenated form.
const UUID_LENGTH: usize = 36;

/// Positions of the hyphens in the hyphenated form.
const HYPHENS: [usize; 4] = [8, 13, 18, 23];

/// A string that is not a UUID in its hyphenated form.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{text:?} is not a UUID")]
pub struct InvalidId {
    /// The rejected input, cut to its first 64 characters.
    pub text: String,
}

/// Checks the hyphenated form and returns it in lower case.
fn canonical_uuid(text: &str) -> Result<String, InvalidId> {
    let invalid = || InvalidId {
        text: text.chars().take(64).collect(),
    };
    if text.len() != UUID_LENGTH {
        return Err(invalid());
    }
    for (index, byte) in text.bytes().enumerate() {
        let expects_hyphen = HYPHENS.contains(&index);
        let is_valid = if expects_hyphen {
            byte == b'-'
        } else {
            byte.is_ascii_hexdigit()
        };
        if !is_valid {
            return Err(invalid());
        }
    }
    Ok(text.to_ascii_lowercase())
}

/// Defines an ID type: a checked UUID string that crosses IPC and the store as
/// a plain string.
macro_rules! uuid_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, specta::Type)]
        pub struct $name(String);

        impl $name {
            /// Checks a UUID in its hyphenated form. Upper-case digits are
            /// accepted and stored in lower case, so equal IDs compare equal.
            pub fn parse(text: &str) -> Result<Self, InvalidId> {
                canonical_uuid(text).map(Self)
            }

            /// The ID in its hyphenated, lower-case form.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                Self::parse(&text).map_err(serde::de::Error::custom)
            }
        }
    };
}

uuid_id!(
    /// Identifies a conversation.
    ConversationId
);
uuid_id!(
    /// Identifies a turn: one request and the agent's response to it.
    TurnId
);
uuid_id!(
    /// Identifies a run: one execution of a turn.
    RunId
);
uuid_id!(
    /// Identifies a part of the transcript.
    PartId
);
uuid_id!(
    /// Identifies a provider profile.
    ProfileId
);
uuid_id!(
    /// Identifies a checkpoint: the summary that stands in for older turns in
    /// the model's context.
    CheckpointId
);
uuid_id!(
    /// Identifies a grant: a permission that lasts for a conversation, for a
    /// project or for every conversation.
    GrantId
);
uuid_id!(
    /// Identifies a command rule.
    RuleId
);
uuid_id!(
    /// Identifies an archived output of a tool call.
    ArchiveId
);
uuid_id!(
    /// Identifies a project: a folder that the owner added, and the
    /// conversations that work in it.
    ProjectId
);
uuid_id!(
    /// Identifies a task: one start or follow-up of a child agent's work,
    /// chosen by the agent that assigns it.
    TaskId
);
uuid_id!(
    /// Identifies a worker that the master paired with.
    WorkerId
);

#[cfg(test)]
mod tests {
    use super::{ConversationId, RunId};

    const SAMPLE: &str = "0199e0f2-7a1b-7c3d-8e4f-0123456789ab";

    #[test]
    fn parse_accepts_the_hyphenated_form_and_lowers_its_case() {
        let id = ConversationId::parse(SAMPLE).unwrap();
        let upper = ConversationId::parse(&SAMPLE.to_ascii_uppercase()).unwrap();

        assert_eq!(id.as_str(), SAMPLE);
        assert_eq!(upper, id);
        assert_eq!(id.to_string(), SAMPLE);
    }

    #[test]
    fn parse_rejects_anything_else() {
        let cases = [
            "",
            "0199e0f27a1b7c3d8e4f0123456789ab",
            "0199e0f2-7a1b-7c3d-8e4f-0123456789a",
            "0199e0f2-7a1b-7c3d-8e4f-0123456789abc",
            "0199e0f2_7a1b-7c3d-8e4f-0123456789ab",
            "0199e0f2-7a1b-7c3d-8e4f-0123456789ag",
            "{0199e0f2-7a1b-7c3d-8e4f-0123456789a}",
            "0199e0f2-7a1b-7c3d-8e4f-01234567890ä",
        ];
        for text in cases {
            assert!(RunId::parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn serde_round_trips_as_a_string_and_checks_the_input() {
        let id = RunId::parse(SAMPLE).unwrap();

        let json = serde_json::to_string(&id).unwrap();
        let back: RunId = serde_json::from_str(&json).unwrap();

        assert_eq!(json, format!("\"{SAMPLE}\""));
        assert_eq!(back, id);
        serde_json::from_str::<RunId>("\"not-an-id\"").unwrap_err();
    }

    #[test]
    fn the_rejected_input_is_cut_in_the_error() {
        let long = "x".repeat(500);

        let error = RunId::parse(&long).unwrap_err();

        assert_eq!(error.text.chars().count(), 64);
    }
}
