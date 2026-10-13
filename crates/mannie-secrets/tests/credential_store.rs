//! V7: keyring-core stores and reads credentials in the operating system's
//! credential store, and without a Secret Service Mannie refuses to store
//! them (ADR-0010).
//!
//! On Windows the round trip runs against Windows Credential Manager on every
//! test run. On Linux it needs an unlocked Secret Service, so those tests are
//! ignored by default; CI runs them with `--ignored` in a session with GNOME
//! Keyring or KWallet, and runs the refusal test in a session without one.

// Integration tests are always built with cfg(test); saying so lets Clippy
// treat the helpers below as test code.
#![cfg(test)]

use mannie_secrets::{CredentialError, CredentialRef, CredentialStore};
use mannie_types::secret::Secret;

/// The service name the tests use, so their entries never mix with the app's.
const SERVICE: &str = "io.github.limbon2.mannie.tests";

/// Deletes the test's credential when the test ends, also when it fails.
struct Cleanup<'a> {
    store: &'a CredentialStore,
    reference: CredentialRef,
}

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        match self.store.delete(&self.reference) {
            Ok(()) | Err(CredentialError::NotFound { .. }) => {}
            Err(error) => eprintln!(
                "could not delete the test credential {}: {error}",
                self.reference
            ),
        }
    }
}

fn unique_reference() -> CredentialRef {
    CredentialRef::parse(&format!("tests/{}", uuid::Uuid::now_v7()))
        .expect("a UUID makes a valid reference")
}

fn round_trip() {
    let store = CredentialStore::open(SERVICE).expect("the credential store opens");
    let reference = unique_reference();
    let cleanup = Cleanup {
        store: &store,
        reference: reference.clone(),
    };

    let missing = store.read(&reference).unwrap_err();
    store
        .save(
            &reference,
            &Secret::new("first-value-0123456789".to_owned()),
        )
        .unwrap();
    let first = store.read(&reference).unwrap();
    store
        .save(
            &reference,
            &Secret::new("second-value-9876543210 ünïcödé".to_owned()),
        )
        .unwrap();
    let second = store.read(&reference).unwrap();
    store.delete(&reference).unwrap();
    let deleted = store.read(&reference).unwrap_err();
    drop(cleanup);

    assert_eq!(missing.code(), "credentials.not_found");
    assert_eq!(first.expose(), "first-value-0123456789");
    assert_eq!(second.expose(), "second-value-9876543210 ünïcödé");
    assert_eq!(deleted.code(), "credentials.not_found");
}

#[cfg(windows)]
#[test]
fn windows_credential_manager_stores_reads_replaces_and_deletes() {
    round_trip();
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "needs an unlocked Secret Service; CI runs it with GNOME Keyring and with KWallet"]
fn the_secret_service_stores_reads_replaces_and_deletes() {
    round_trip();
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "needs a session without a Secret Service; CI runs it on a session bus that has none"]
fn without_a_secret_service_no_credential_is_stored() {
    let error = CredentialStore::open(SERVICE).unwrap_err();

    assert!(
        matches!(
            error,
            CredentialError::Unavailable {
                reason: mannie_secrets::UnavailableReason::NoSecretService,
                ..
            }
        ),
        "{error:?}"
    );
    assert_eq!(error.code(), "credentials.unavailable");
}
