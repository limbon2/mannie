//! The dispatch's log (ADR-0034, decision 8): a call leaves the plugin, the
//! command, the outcome's code and the duration, and never the input or the
//! output, and an input that does not decode leaves only its category,
//! because the decoder's message can quote the input (T-6).
//!
//! This test has a binary of its own: tracing caches which callsites are
//! enabled across the whole process, so a capturing subscriber shares it with
//! no other test.

#![cfg(test)]

use std::io::Write;
use std::sync::{Arc, Mutex};

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::{HostInfo, HostProfile, StartContext};
use mannie_plugin_sdk::contracts::{Clock, UtcMillis};
use mannie_plugin_sdk::error::StartFailure;
use mannie_plugin_sdk::ipc::{
    CommandSpec, Handlers, HostRefusal, InputRefusal, PluginError, Validate,
};
use mannie_plugin_sdk::plugin::{Instance, Plugin, PluginEntry, Started};
use mannie_plugins::{CallFailure, PluginHost, PluginHostDeps, PluginList};
use mannie_store::{Store, StoreOptions};
use mannie_types::clock::ManualClock;
use serde::{Deserialize, Serialize};
use serde_json::json;

const PLANTED_INPUT: &str = "sk-planted-dispatch-input-0123456789";
const PLANTED_OUTPUT: &str = "planted-dispatch-output-9876543210";

/// Collects what the tracing subscriber writes.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
struct Echo {
    text: String,
}

impl Validate for Echo {
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

#[derive(Debug, Serialize, specta::Type, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
enum EchoError {
    #[error("the echo failed")]
    Failed,
}

impl PluginError for EchoError {
    fn code(&self) -> &'static str {
        "echo.failed"
    }
}

enum EchoCommand {}

impl CommandSpec for EchoCommand {
    const NAME: &'static str = "echo";
    const DOCS: &'static str = "Answers with the planted output.";
    type Input = Echo;
    type Output = String;
    type Error = EchoError;
}

#[derive(Debug)]
struct EchoPlugin;

#[derive(Debug)]
struct EchoInstance;

impl Instance for EchoInstance {
    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(std::future::ready(()))
    }
}

impl Plugin for EchoPlugin {
    fn start(&self, _context: StartContext) -> BoxFuture<'static, Result<Started, StartFailure>> {
        let handlers = Handlers::new().command::<EchoCommand, _, _>(|input: Echo| async move {
            if input.text.is_empty() {
                return Err(EchoError::Failed);
            }
            Ok(PLANTED_OUTPUT.to_owned())
        });
        Box::pin(std::future::ready(Ok(
            Started::new(EchoInstance).with_handlers(handlers)
        )))
    }
}

const MANIFEST: &str =
    "# The test's plugin.\nid = \"echo\"\nsdk = 1\norder = 10\n\n[rust]\nentry = \"mannie-echo\"\n";

#[tokio::test]
async fn a_call_logs_its_plugin_command_outcome_and_duration_and_never_its_input_or_output() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let directory = tempfile::tempdir().unwrap();
    let hosted: Arc<dyn Plugin> = Arc::new(EchoPlugin);
    let list = PluginList::new(vec![PluginEntry::new(MANIFEST, hosted)]).unwrap();
    let clock: Arc<dyn Clock> = Arc::new(ManualClock::new(UtcMillis::from_millis(1)));
    let options = StoreOptions::new(directory.path().join("backups"), Arc::clone(&clock));
    let store = Store::open(&directory.path().join("mannie.sqlite"), &options).unwrap();
    let host = PluginHost::new(PluginHostDeps {
        store: store.clone(),
        plugins: list,
        providers: mannie_store::service_providers(&store),
        consumers: Vec::new(),
        clock,
        host: HostInfo::new("0.3.0", HostProfile::Validation),
    })
    .unwrap();
    host.start_chosen().await;
    host.set_enabled("echo", true).await.unwrap();

    let answered = host
        .invoke("echo", "echo", json!({ "text": PLANTED_INPUT }))
        .await;
    let unknown_field = host
        .invoke(
            "echo",
            "echo",
            json!({ "text": "x", "token": PLANTED_INPUT }),
        )
        .await;
    let wrong_type = host
        .invoke("echo", "echo", json!({ "text": [PLANTED_INPUT] }))
        .await;
    let failed = host.invoke("echo", "echo", json!({ "text": "" })).await;

    let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert_eq!(answered, Ok(json!(PLANTED_OUTPUT)));
    assert_eq!(
        unknown_field,
        Err(CallFailure::Host(HostRefusal::InvalidInput { field: None }))
    );
    assert_eq!(
        wrong_type,
        Err(CallFailure::Host(HostRefusal::InvalidInput { field: None }))
    );
    assert_eq!(failed.unwrap_err().code(), "echo.failed");
    assert!(
        log.contains("plugin=\"echo\" command=\"echo\" outcome=\"ok\" duration_ms="),
        "{log}"
    );
    assert!(log.contains("outcome=\"invalid_input\""), "{log}");
    assert!(log.contains("outcome=\"echo.failed\""), "{log}");
    assert!(log.contains("category=\"data\""), "{log}");
    assert!(!log.contains(PLANTED_INPUT), "{log}");
    assert!(!log.contains(PLANTED_OUTPUT), "{log}");
    assert!(!log.contains("token"), "{log}");
}
