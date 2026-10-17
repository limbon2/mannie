//! The Workers plugin through the plugin host (ADR-0030, ADR-0031;
//! docs/features/workers/plugin.md): its default follows the paired workers,
//! turning it on starts its hub and offers `delegate_task` while a worker is
//! paired, turning it off refuses its commands with `plugin_off` and ends
//! the list of workers, and a start without the services it needs fails.

#![cfg(test)]

use std::sync::Arc;

use mannie_link::host::WorkerDetails;
use mannie_plugins::{GRANT_MISSING, Grants, HostedPlugin, Plugin, PluginHost};
use mannie_runtime::testing::{Harness, HarnessOptions, reply};
use mannie_types::chat::StartConversation;
use mannie_types::clock::SystemClock;
use mannie_types::ids::WorkerId;
use mannie_types::plugins::PluginState;
use mannie_workers::commands::WorkersError;
use mannie_workers::store::{NewPairedWorker, WORKERS_MODULE, WorkersStore};
use mannie_workers::testing::MemorySecrets;
use mannie_workers::{DELEGATE_TASK, WORKERS_KIND, WorkersPlugin};

async fn harness() -> Harness {
    Harness::with_options(HarnessOptions {
        use_tools: true,
        store_modules: vec![WORKERS_MODULE],
        ..HarnessOptions::default()
    })
    .await
}

/// The grants of the desktop app's composition root.
fn grants(harness: &Harness) -> Grants {
    Grants {
        executors: Some(harness.runtime.executor_services()),
        credentials: Some(harness.runtime.credential_lender()),
        secrets: Some(Arc::new(MemorySecrets::default())),
    }
}

fn host(harness: &Harness, plugin: &WorkersPlugin, grants: Grants) -> PluginHost {
    let plugin: Arc<dyn Plugin> = Arc::new(plugin.clone());
    PluginHost::new(
        harness.runtime.clone(),
        harness.store.clone(),
        vec![HostedPlugin { plugin, grants }],
        Arc::new(SystemClock),
        env!("CARGO_PKG_VERSION").to_owned(),
    )
    .unwrap()
}

async fn pair_a_worker(harness: &Harness) {
    let workers = WorkersStore::new(harness.store.module(&WORKERS_MODULE).unwrap());
    workers
        .add_paired_worker(NewPairedWorker {
            id: WorkerId::parse("0199e0f2-0000-7000-b000-000000000001").unwrap(),
            name: "build-01".to_owned(),
            chosen_name: None,
            // Nothing listens there, so the worker stays offline.
            address: "127.0.0.1:9".to_owned(),
            key_pin: "a".repeat(64),
            details: WorkerDetails::default(),
        })
        .await
        .unwrap();
}

/// Whether a run that starts now is offered `delegate_task`.
async fn offers_delegate_task(harness: &Harness, marker: &str) -> bool {
    harness.model.push_reply(reply("Hello."));
    let started = harness
        .runtime
        .start_conversation(StartConversation {
            mentions: Vec::new(),
            text: format!("{marker}: hello"),
            profile_id: None,
            project_id: None,
        })
        .await
        .unwrap();
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    harness.model.requests_for(marker)[0]
        .tools
        .iter()
        .any(|tool| tool.name == DELEGATE_TASK)
}

#[tokio::test]
async fn without_a_paired_worker_workers_starts_off_and_its_commands_say_so() {
    let harness = harness().await;
    let plugin = WorkersPlugin::new();
    let host = host(&harness, &plugin, grants(&harness));

    host.start_chosen().await;

    assert_eq!(host.views()[0].state, PluginState::Off);
    assert_eq!(plugin.hub().unwrap_err(), WorkersError::PluginOff);
    assert_eq!(plugin.subscribe().unwrap_err(), WorkersError::PluginOff);
    assert!(!offers_delegate_task(&harness, "OFF").await);
}

#[tokio::test]
async fn with_a_paired_worker_workers_starts_on_and_turning_it_off_ends_its_parts() {
    let harness = harness().await;
    pair_a_worker(&harness).await;
    let plugin = WorkersPlugin::new();
    let host = host(&harness, &plugin, grants(&harness));

    host.start_chosen().await;
    let on = host.views()[0].clone();
    let workers = plugin.hub().unwrap().worker_views().borrow().clone();
    let mut subscription = plugin.subscribe().unwrap();
    let offered = offers_delegate_task(&harness, "ON").await;
    let off = host.set_enabled(WORKERS_KIND, false).await.unwrap();

    assert_eq!(on.state, PluginState::On);
    assert!(on.enabled, "the default is kept as the owner's choice");
    assert_eq!(workers.len(), 1);
    assert!(offered);
    assert_eq!(off.state, PluginState::Off);
    subscription.on.changed().await.unwrap();
    assert!(!*subscription.on.borrow(), "the list of workers ends");
    assert_eq!(plugin.hub().unwrap_err(), WorkersError::PluginOff);
    assert!(!offers_delegate_task(&harness, "OFF").await);
    // The paired worker stays for the next start.
    let again = host.set_enabled(WORKERS_KIND, true).await.unwrap();
    assert_eq!(again.state, PluginState::On);
    assert_eq!(plugin.hub().unwrap().worker_views().borrow().len(), 1);
}

#[tokio::test]
async fn a_start_without_the_services_that_workers_needs_fails_and_mannie_goes_on() {
    let harness = harness().await;
    let plugin = WorkersPlugin::new();
    let host = host(&harness, &plugin, Grants::default());

    let view = host.set_enabled(WORKERS_KIND, true).await.unwrap();

    assert_eq!(view.state, PluginState::Failed);
    assert_eq!(view.failure.as_deref(), Some(GRANT_MISSING));
    assert_eq!(plugin.hub().unwrap_err(), WorkersError::PluginOff);
    assert!(!offers_delegate_task(&harness, "FAILED").await);
}
