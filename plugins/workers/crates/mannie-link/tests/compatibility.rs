//! The documents of the link (V6; plugins/workers/docs/
//! compatibility-and-updates.md, documents). The link carries documents of
//! `mannie-types` as JSON inside its protobuf messages, where `buf breaking`
//! does not see them, so fixtures guard them: `tests/fixtures/documents/`
//! keeps each document as a minor version writes it. This version reads the
//! documents of every earlier version without losing a field, writes the
//! newest ones exactly, and ignores the fields that a later version adds,
//! which is what a worker one minor version behind needs from both sides.
//!
//! A release that changes a document adds a folder for its minor version
//! with every document, and makes it `NEWEST`. The older folders never
//! change.

#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use mannie_link::host::WorkerDetails;
use mannie_plugin_harness::runtime::{AssignError, LeaseError};
use mannie_plugin_sdk::contracts::{AssignedTaskState, TaskAssignment};
use mannie_plugin_sdk::contracts::{ChatError, CheckpointRef, EarlierTurns, LoadEarlierTurns};
use mannie_plugin_sdk::contracts::{Checkpoint, ConversationUpdate};
use mannie_plugin_sdk::contracts::{DecideApproval, ReadToolOutput, ToolOutputPage, ToolsError};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// The folder of the documents that this version writes.
const NEWEST: &str = "v0.4";

/// The codes of the refusals that cross the link without a document.
const REFUSAL_CODES: &str = "refusal-codes.json";

/// A document of the link: its fixture and how this version reads it and
/// writes it back.
struct Document {
    file: &'static str,
    read_and_write: fn(&Value) -> Result<Value, serde_json::Error>,
}

fn read_and_write<T: DeserializeOwned + Serialize>(
    document: &Value,
) -> Result<Value, serde_json::Error> {
    let read: T = serde_json::from_value(document.clone())?;
    serde_json::to_value(&read)
}

fn document<T: DeserializeOwned + Serialize>(file: &'static str) -> Document {
    Document {
        file,
        read_and_write: read_and_write::<T>,
    }
}

/// Every document that crosses the link, in either direction.
fn documents() -> Vec<Document> {
    vec![
        document::<TaskAssignment>("task-assignment.json"),
        document::<AssignedTaskState>("assigned-task-state-running.json"),
        document::<AssignedTaskState>("assigned-task-state-reported.json"),
        document::<WorkerDetails>("worker-details.json"),
        document::<ConversationUpdate>("conversation-update-snapshot.json"),
        document::<Vec<ConversationUpdate>>("conversation-update-events.json"),
        document::<DecideApproval>("decide-approval.json"),
        document::<LoadEarlierTurns>("load-earlier-turns.json"),
        document::<EarlierTurns>("earlier-turns.json"),
        document::<CheckpointRef>("checkpoint-ref.json"),
        document::<Checkpoint>("checkpoint.json"),
        document::<ReadToolOutput>("read-tool-output.json"),
        document::<ToolOutputPage>("tool-output-page.json"),
        document::<Vec<ChatError>>("chat-errors.json"),
        document::<Vec<ToolsError>>("tools-errors.json"),
    ]
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/documents")
}

/// The folders of the fixtures, one for each minor version.
fn versions() -> Vec<String> {
    let mut versions: Vec<String> = fs::read_dir(fixtures())
        .expect("the fixtures exist")
        .map(|entry| {
            entry
                .expect("a fixture folder")
                .file_name()
                .into_string()
                .expect("a folder name in UTF-8")
        })
        .collect();
    versions.sort();
    versions
}

fn load(version: &str, file: &str) -> Option<Value> {
    let path = fixtures().join(version).join(file);
    let text = fs::read(&path).ok()?;
    Some(
        serde_json::from_slice(&text)
            .unwrap_or_else(|error| panic!("{version}/{file} is not JSON: {error}")),
    )
}

/// Whether `written` holds every field of `original` with its value. It may
/// hold more: the fields that a later version added.
fn keeps_every_field(original: &Value, written: &Value) -> bool {
    match (original, written) {
        (Value::Object(original), Value::Object(written)) => original.iter().all(|(key, value)| {
            written
                .get(key)
                .is_some_and(|kept| keeps_every_field(value, kept))
        }),
        (Value::Array(original), Value::Array(written)) => {
            original.len() == written.len()
                && original
                    .iter()
                    .zip(written)
                    .all(|(value, kept)| keeps_every_field(value, kept))
        }
        _ => original == written,
    }
}

/// The document with a field in every object that a later version added.
fn with_later_fields(document: &Value) -> Value {
    match document {
        Value::Object(fields) => {
            let mut fields: Map<String, Value> = fields
                .iter()
                .map(|(key, value)| (key.clone(), with_later_fields(value)))
                .collect();
            fields.insert("addedByALaterVersion".to_owned(), Value::Bool(true));
            Value::Object(fields)
        }
        Value::Array(items) => Value::Array(items.iter().map(with_later_fields).collect()),
        other => other.clone(),
    }
}

fn newest(document: &Document) -> Value {
    load(NEWEST, document.file).unwrap_or_else(|| {
        panic!(
            "{NEWEST}/{} is missing: every document of the link has a fixture",
            document.file
        )
    })
}

#[test]
fn this_version_writes_the_newest_documents_exactly() {
    for document in documents() {
        let fixture = newest(&document);

        let written = (document.read_and_write)(&fixture)
            .unwrap_or_else(|error| panic!("{}: {error}", document.file));

        assert_eq!(written, fixture, "{}", document.file);
    }
}

#[test]
fn this_version_reads_every_earlier_versions_documents_without_losing_a_field() {
    for version in versions() {
        for document in documents() {
            let Some(fixture) = load(&version, document.file) else {
                continue;
            };

            let written = (document.read_and_write)(&fixture)
                .unwrap_or_else(|error| panic!("{version}/{}: {error}", document.file));

            assert!(
                keeps_every_field(&fixture, &written),
                "{version}/{} lost a field: {written}",
                document.file
            );
        }
    }
}

#[test]
fn the_fields_that_a_later_version_adds_are_ignored() {
    for document in documents() {
        let fixture = newest(&document);

        let written = (document.read_and_write)(&with_later_fields(&fixture))
            .unwrap_or_else(|error| panic!("{}: {error}", document.file));

        assert_eq!(written, fixture, "{}", document.file);
    }
}

#[test]
fn every_fixture_is_a_document_that_this_test_knows() {
    let known: BTreeSet<&str> = documents()
        .iter()
        .map(|document| document.file)
        .chain([REFUSAL_CODES])
        .collect();
    let versions = versions();

    assert!(versions.contains(&NEWEST.to_owned()), "{versions:?}");
    for version in versions {
        for entry in fs::read_dir(fixtures().join(&version)).expect("a fixture folder") {
            let name = entry
                .expect("a fixture")
                .file_name()
                .into_string()
                .expect("a file name in UTF-8");
            assert!(
                known.contains(name.as_str()),
                "{version}/{name} is not a document of the link"
            );
        }
    }
}

fn listed(codes: &Value, kind: &str) -> BTreeSet<String> {
    codes[kind]
        .as_array()
        .unwrap_or_else(|| panic!("the refusal codes list the {kind} codes"))
        .iter()
        .map(|code| code.as_str().expect("a code").to_owned())
        .collect()
}

#[test]
fn this_version_knows_the_refusal_codes_of_every_earlier_version() {
    let assign: BTreeSet<String> = [
        AssignError::NotAWorker,
        AssignError::Conflict,
        AssignError::ConversationNotFound,
        AssignError::StillRunning,
        AssignError::Invalid {
            detail: String::new(),
        },
        AssignError::ShuttingDown,
        AssignError::StorageFailed,
    ]
    .iter()
    .map(|error| error.code().to_owned())
    .collect();
    let lease: BTreeSet<String> = [
        LeaseError::TaskNotActive,
        LeaseError::BindingMismatch,
        LeaseError::VaultUnavailable,
        LeaseError::StorageFailed,
    ]
    .into_iter()
    .map(|error| error.code().to_owned())
    .collect();

    for version in versions() {
        let Some(codes) = load(&version, REFUSAL_CODES) else {
            continue;
        };
        assert!(
            listed(&codes, "assign").is_subset(&assign),
            "{version}: {codes}"
        );
        assert!(
            listed(&codes, "lease").is_subset(&lease),
            "{version}: {codes}"
        );
    }
    let newest = load(NEWEST, REFUSAL_CODES).expect("the newest refusal codes");
    assert_eq!(listed(&newest, "assign"), assign);
    assert_eq!(listed(&newest, "lease"), lease);
}
