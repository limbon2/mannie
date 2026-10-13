//! The M0 planted-secret test (security architecture, rules for handling
//! secrets): a known test secret goes through the credential store, the
//! logger, the store and a crash, and afterwards no log, database or crash
//! file contains it.
//!
//! The logger is process-wide, so this file has one test. On Linux the
//! credential store part needs a Secret Service; CI runs this test in a session
//! that has one, and without one the test still checks the other parts.

// Integration tests are always built with cfg(test); saying so lets Clippy
// treat the helpers below as test code.
#![cfg(test)]

use std::fs;
use std::path::Path;
use std::sync::Arc;

use mannie_logging::{LogConfig, Redactor};
use mannie_secrets::{CredentialError, CredentialRef, CredentialStore};
use mannie_store::{Store, StoreOptions};
use mannie_types::clock::SystemClock;
use mannie_types::interface::{InterfaceSettings, LocaleTag};
use mannie_types::secret::Secret;

const SERVICE: &str = "io.github.limbon2.mannie.tests";
const PLANTED: &str = "planted-7c1e5b9d-provider-key-for-tests";
const TARGET: &str = "mannie_planted_secret";

/// Every file below `directory`, with its bytes.
fn files_below(directory: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory).expect("the folder can be listed") {
        let path = entry.expect("the folder can be listed").path();
        if path.is_dir() {
            files.extend(files_below(&path));
        } else {
            let bytes = fs::read(&path).expect("the file can be read");
            files.push((path.display().to_string(), bytes));
        }
    }
    files
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Saves the planted secret in the credential store and reads it back, as the
/// master does with a provider credential. Returns `None` when the platform
/// has no credential store in this session.
fn through_the_credential_store(redactor: &Redactor) -> Option<Secret> {
    let store = match CredentialStore::open(SERVICE) {
        Ok(store) => store,
        Err(CredentialError::Unavailable { .. }) if cfg!(not(windows)) => {
            eprintln!(
                "no credential store in this session; checking the logger, the store and crash files only"
            );
            return None;
        }
        Err(error) => panic!("the credential store does not open: {error}"),
    };
    let reference = CredentialRef::parse(&format!("tests/{}", uuid::Uuid::now_v7()))
        .expect("a valid reference");
    store
        .save(&reference, &Secret::new(PLANTED.to_owned()))
        .expect("the credential saves");
    let secret = store.read(&reference).expect("the credential reads back");
    redactor.register(&secret);
    let missing = CredentialRef::parse("tests/missing").expect("a valid reference");
    let error = store.read(&missing).unwrap_err();
    tracing::warn!(target: TARGET, ?store, %error, error.code = error.code(), "reading a missing credential failed");
    store.delete(&reference).expect("the credential deletes");
    Some(secret)
}

#[tokio::test]
async fn a_planted_secret_reaches_no_log_database_or_crash_file() {
    let directory = tempfile::tempdir().unwrap();
    let logs = directory.path().join("logs");
    let data = directory.path().join("data");
    let redactor = Redactor::new();
    let guard = mannie_logging::init(
        &LogConfig::new(logs.clone(), "mannie", false),
        &redactor,
        Arc::new(SystemClock),
    )
    .unwrap();

    // The Secret Service store blocks on its own runtime, which must not run
    // on this test's runtime thread.
    let credential_redactor = redactor.clone();
    let stored =
        tokio::task::spawn_blocking(move || through_the_credential_store(&credential_redactor))
            .await
            .expect("the credential store part finished");
    let secret = stored.unwrap_or_else(|| {
        let secret = Secret::new(PLANTED.to_owned());
        redactor.register(&secret);
        secret
    });

    let store = Store::open(
        &data.join("mannie.sqlite"),
        &StoreOptions::new(data.join("backups"), Arc::new(SystemClock)),
    )
    .expect("the store opens");
    store
        .save_settings(&InterfaceSettings {
            locale: Some(LocaleTag::parse("en").unwrap()),
        })
        .await
        .expect("the settings save");
    tracing::info!(target: TARGET, ?secret, "the secret through its Debug format");
    tracing::error!(target: TARGET, "a bug: the exposed secret in a message: {}", secret.expose());
    tracing::error!(target: TARGET, header = %format!("Bearer {}", secret.expose()), "a bug: the secret in a header");
    let exposed = secret.expose().to_owned();
    let panicked =
        std::thread::spawn(move || panic!("a bug: the secret in a panic: {exposed}")).join();
    drop(std::panic::take_hook());
    assert!(panicked.is_err());
    drop(store);
    drop(guard);

    let files: Vec<_> = files_below(&logs)
        .into_iter()
        .chain(files_below(&data))
        .collect();
    assert!(
        files.iter().any(|(name, _)| name.contains("crash-")),
        "no crash file: {:?}",
        names(&files)
    );
    assert!(
        files
            .iter()
            .any(|(name, _)| name.ends_with("mannie.sqlite")),
        "no database: {:?}",
        names(&files)
    );
    for (name, bytes) in &files {
        assert!(
            !contains(bytes, PLANTED.as_bytes()),
            "{name} contains the planted secret"
        );
    }
}

fn names(files: &[(String, Vec<u8>)]) -> Vec<&str> {
    files.iter().map(|(name, _)| name.as_str()).collect()
}
