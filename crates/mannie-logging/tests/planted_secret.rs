//! Planted-secret test for logging (security architecture, rules for handling
//! secrets): a known test secret is logged in every way a bug could log it,
//! and a thread panics with it in its message. Afterwards no log file and no
//! crash file contains it.
//!
//! `init` installs the process-wide logger, so this file has one test.

// Integration tests are always built with cfg(test); saying so lets Clippy
// treat the helpers below as test code.
#![cfg(test)]

use std::fs;
use std::path::Path;
use std::sync::Arc;

use mannie_logging::{LogConfig, REDACTED, Redactor, init};
use mannie_types::clock::SystemClock;
use mannie_types::secret::Secret;

/// The planted secret. The second form looks like an OpenAI key but is never
/// registered, so only the credential patterns can catch it.
const PLANTED: &str = "planted-0f3c9e7a-test-secret-value";
const UNREGISTERED_KEY: &str = "sk-proj-unregistered0123456789abcdef";

/// Mannie's crates log at `info`; the test logs under a target of the same kind.
const TARGET: &str = "mannie_planted_secret";

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "the test reads the fields only through the Debug format, which it logs"
)]
struct ProviderRequest {
    url: String,
    authorization: String,
    credential: Secret,
}

/// The name and contents of every file in the log folder.
fn files_in(directory: &Path) -> Vec<(String, String)> {
    fs::read_dir(directory)
        .expect("the log folder exists")
        .map(|entry| {
            let path = entry.expect("the log folder can be listed").path();
            let name = path
                .file_name()
                .expect("a listed file has a name")
                .to_string_lossy()
                .into_owned();
            (
                name,
                fs::read_to_string(&path).expect("log and crash files are UTF-8 text"),
            )
        })
        .collect()
}

#[test]
fn a_planted_secret_reaches_no_log_or_crash_file() {
    let directory = tempfile::tempdir().unwrap();
    let logs = directory.path().join("logs");
    let redactor = Redactor::new();
    let guard = init(
        &LogConfig::new(logs.clone(), "mannie-test", false),
        &redactor,
        Arc::new(SystemClock),
    )
    .unwrap();

    let secret = Secret::new(PLANTED.to_owned());
    redactor.register(&secret);
    let request = ProviderRequest {
        url: format!(
            "https://api.example.test/v1/responses?key={}",
            secret.expose()
        ),
        authorization: format!("Bearer {}", secret.expose()),
        credential: secret.clone(),
    };

    tracing::info!(target: TARGET, ?secret, "the secret through its Debug format");
    tracing::info!(target: TARGET, value = secret.expose(), "a bug: the exposed value as a field");
    tracing::warn!(target: TARGET, "a bug: the exposed value in a message: {}", secret.expose());
    tracing::error!(target: TARGET, ?request, "a bug: a request with the secret in its URL and header");
    tracing::info!(target: TARGET, api_key = UNREGISTERED_KEY, "a bug: an unregistered key in a credential field");
    tracing::info!(target: TARGET, "a bug: an unregistered key in a message: {UNREGISTERED_KEY}");
    let span = tracing::info_span!(target: TARGET, "provider_call", authorization = %request.authorization);
    span.in_scope(|| tracing::info!(target: TARGET, "inside a span whose field holds the secret"));

    let exposed = secret.expose().to_owned();
    let panicked =
        std::thread::spawn(move || panic!("a bug: the secret in a panic message: {exposed}"))
            .join();
    // Back to the default hook, so a failing assertion below prints its message.
    drop(std::panic::take_hook());
    assert!(panicked.is_err());
    drop(guard);

    let files = files_in(&logs);
    let crash_files: Vec<_> = files
        .iter()
        .filter(|(name, _)| name.starts_with("crash-"))
        .collect();
    let log_files: Vec<_> = files
        .iter()
        .filter(|(name, _)| name.starts_with("mannie-test."))
        .collect();
    assert_eq!(crash_files.len(), 1, "{files:#?}");
    assert_eq!(log_files.len(), 1, "{files:#?}");
    for (name, contents) in &files {
        assert!(
            !contents.contains(PLANTED),
            "{name} contains the planted secret:\n{contents}"
        );
        assert!(
            !contents.contains(UNREGISTERED_KEY),
            "{name} contains the unregistered key:\n{contents}"
        );
    }
    let log = &log_files[0].1;
    // Seven events, the panic, and nothing else.
    assert_eq!(log.lines().count(), 8, "{log}");
    assert!(log.contains("Secret(<redacted>)"), "{log}");
    assert!(log.contains(REDACTED), "{log}");
    assert!(
        crash_files[0]
            .1
            .contains("a bug: the secret in a panic message"),
        "{}",
        crash_files[0].1
    );
}
