//! The documents that cross the link as JSON inside protobuf messages, and
//! how errors cross it as gRPC statuses (docs/features/workers/
//! pairing-and-link.md, the services).
//!
//! A worker's refusals travel as `FAILED_PRECONDITION` with the error's
//! stable code, or with the error's JSON for the errors of chat and tools,
//! which the master hands on as they are. Everything else that fails on the
//! way means that the worker could not be reached.

use serde::Serialize;
use serde::de::DeserializeOwned;
use tonic::{Code, Status};

/// The JSON of a document, for a message.
pub(crate) fn to_json<T: Serialize>(value: &T) -> Result<String, Status> {
    serde_json::to_string(value).map_err(|error| {
        tracing::error!(%error, "a document of the link could not be written");
        Status::internal("a document could not be written")
    })
}

/// A document from a message. A document that does not parse is the
/// sender's error.
pub(crate) fn from_json<T: DeserializeOwned>(text: &str, what: &str) -> Result<T, Status> {
    serde_json::from_str(text).map_err(|error| {
        tracing::warn!(%error, what, "a document of the link is not valid");
        Status::invalid_argument(format!("{what} is not valid"))
    })
}

/// A worker's refusal with its error's stable code.
pub(crate) fn refused(code: &str) -> Status {
    Status::failed_precondition(code)
}

/// A worker's refusal with its error as JSON, for the errors of chat and
/// tools.
pub(crate) fn refused_with<T: Serialize>(error: &T) -> Status {
    match serde_json::to_string(error) {
        Ok(json) => Status::failed_precondition(json),
        Err(_unwritable) => Status::internal("an error could not be written"),
    }
}

/// What a status from a worker means to the master.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Answer {
    /// The worker refused, with its message: a code, or an error's JSON.
    Refused(String),
    /// The worker could not be reached, or the call failed on the way.
    Unreachable,
}

/// Reads a status from a worker.
pub(crate) fn answer(status: &Status) -> Answer {
    match status.code() {
        Code::FailedPrecondition => Answer::Refused(status.message().to_owned()),
        _other => Answer::Unreachable,
    }
}
