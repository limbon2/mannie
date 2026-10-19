//! The Workers plugin through the plugin host and its dispatch (ADR-0030,
//! ADR-0031, ADR-0034; plugins/workers/docs/plugin.md): its exported
//! manifest and interface, its default follows the paired workers, turning
//! it on starts its hub, offers `delegate_task` while a worker is paired and
//! lists the workers on its channel, turning it off makes the host refuse
//! its commands with `plugin_off` and end the channel, a start on a host
//! without a service that it declares fails while Mannie goes on, and it
//! keeps the rules of the SDK.

#![cfg(test)]

use std::sync::Arc;

use futures_util::StreamExt;
use mannie_link::host::WorkerDetails;
use mannie_plugin_harness::conformance::conformance;
use mannie_plugin_harness::host::{CallFailure, HostOptions, HostedPlugin, PluginHost};
use mannie_plugin_harness::runtime::{Harness, HarnessOptions, reply};
use mannie_plugin_harness::types::{PluginState, StartConversation, SystemClock};
use mannie_plugin_sdk::contracts::{SubscriptionEnd, WorkerId};
use mannie_plugin_sdk::error::SERVICE_UNAVAILABLE;
use mannie_plugin_sdk::host::ServiceProvider;
use mannie_plugin_sdk::ipc::{ChannelMessage, HostRefusal, PluginStream};
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::testing::FakeSecrets;
use mannie_workers::keys::MASTER_KEY_ENTRY;
use mannie_workers::store::{NewPairedWorker, WORKERS_MODULE};
use mannie_workers::{DELEGATE_TASK, PLUGIN_MANIFEST, WORKERS_KIND};
use mannie_workers_testing::master::workers_store;
use serde_json::{Value, json};

/// The host of the desktop app, with tools, and with the credential store
/// in memory when `secrets` holds.
fn options(secrets: bool) -> HostOptions {
    let options = HostOptions::new().with_core(HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    });
    if !secrets {
        return options;
    }
    let provider: Arc<dyn ServiceProvider> = Arc::new(FakeSecrets::new());
    options.with_provider(provider)
}

/// Workers on that host with its exported manifest, as the desktop app
/// lists it, before the host starts the plugins that are on.
async fn hosted(secrets: bool) -> (Harness, PluginHost) {
    HostedPlugin::new(mannie_workers::mannie_plugin(), options(secrets))
        .await
        .into_parts()
}

/// The answer of the host while Workers is not on.
fn plugin_off() -> CallFailure {
    CallFailure::Host(HostRefusal::plugin_off(WORKERS_KIND))
}

/// A command that reaches the hub, with a worker that is not paired.
async fn rename(host: &PluginHost) -> Result<Value, CallFailure> {
    host.invoke(
        WORKERS_KIND,
        "rename_worker",
        json!({ "workerId": "0199e0f2-0000-7000-b000-000000000009", "name": "build" }),
    )
    .await
}

async fn follow(host: &PluginHost) -> Result<PluginStream<ChannelMessage<Value>>, CallFailure> {
    host.open_channel(WORKERS_KIND, "subscribe_workers", Value::Null)
        .await
}

/// The number of workers in the next list of the channel.
async fn next_count(channel: &mut PluginStream<ChannelMessage<Value>>) -> usize {
    match channel.next().await {
        Some(ChannelMessage::Item { item }) => {
            assert_eq!(item["type"], "workers", "{item}");
            item["workers"].as_array().unwrap().len()
        }
        other => panic!("not a list of workers: {other:?}"),
    }
}

async fn pair_a_worker(harness: &Harness) {
    let workers = workers_store(&harness.store, Arc::new(SystemClock));
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
        .start_conversation(StartConversation::new(format!("{marker}: hello")))
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

#[test]
fn the_exported_entry_carries_the_manifest_of_the_plugins_folder() {
    let manifest = Manifest::parse(PLUGIN_MANIFEST).unwrap();
    let entry = mannie_workers::mannie_plugin();
    let services: Vec<(&str, u16)> = manifest
        .services
        .iter()
        .map(|(name, major)| (name.as_str(), *major))
        .collect();

    assert_eq!(manifest.id.as_str(), WORKERS_KIND);
    assert_eq!(manifest.sdk, 1);
    assert_eq!(manifest.rust.entry, env!("CARGO_PKG_NAME"));
    assert_eq!(
        services,
        [
            ("credentials", 1),
            ("executor-host", 1),
            ("secrets", 1),
            ("store", 1)
        ]
    );
    assert_eq!(manifest.secrets.legacy, [MASTER_KEY_ENTRY]);
    assert_eq!(manifest.secrets.entries, Vec::<String>::new());
    assert_eq!(WORKERS_MODULE.name, manifest.id.prefix());
    assert_eq!(entry.manifest, PLUGIN_MANIFEST);
    assert_eq!(entry.plugin.store_module(), Some(WORKERS_MODULE));
    assert_eq!(
        entry.plugin.interface().command_names(),
        ["pair_worker", "rename_worker", "remove_worker"]
    );
    assert_eq!(
        entry.plugin.interface().channel_names(),
        ["subscribe_workers"]
    );
}

#[tokio::test]
async fn without_a_paired_worker_workers_starts_off_and_its_commands_say_so() {
    let (harness, host) = hosted(true).await;

    host.start_chosen().await;

    assert_eq!(host.views()[0].state, PluginState::Off);
    assert_eq!(rename(&host).await, Err(plugin_off()));
    assert_eq!(follow(&host).await.err(), Some(plugin_off()));
    assert!(!offers_delegate_task(&harness, "OFF").await);
}

#[tokio::test]
async fn with_a_paired_worker_workers_starts_on_and_turning_it_off_ends_its_parts() {
    let (harness, host) = hosted(true).await;
    pair_a_worker(&harness).await;

    host.start_chosen().await;
    let on = host.views()[0].clone();
    let mut channel = follow(&host).await.unwrap();
    let workers = next_count(&mut channel).await;
    let renamed = rename(&host).await;
    let offered = offers_delegate_task(&harness, "ON").await;
    let off = host.set_enabled(WORKERS_KIND, false).await.unwrap();

    assert_eq!(on.state, PluginState::On);
    assert!(on.enabled, "the default is kept as the owner's choice");
    assert_eq!(workers, 1);
    assert_eq!(
        renamed.err().map(|failure| failure.code()),
        Some("workers.worker_not_found"),
        "the hub answers while Workers is on"
    );
    assert!(offered);
    assert_eq!(off.state, PluginState::Off);
    assert_eq!(
        channel.next().await,
        Some(ChannelMessage::Ended {
            reason: SubscriptionEnd::PluginOff
        }),
        "the list of workers ends"
    );
    assert_eq!(rename(&host).await, Err(plugin_off()));
    assert!(!offers_delegate_task(&harness, "OFF").await);
    // The paired worker stays for the next start.
    let again = host.set_enabled(WORKERS_KIND, true).await.unwrap();
    assert_eq!(again.state, PluginState::On);
    assert_eq!(next_count(&mut follow(&host).await.unwrap()).await, 1);
}

#[tokio::test]
async fn a_start_on_a_host_without_a_secrets_provider_fails_with_service_unavailable_and_mannie_goes_on()
 {
    let (harness, host) = hosted(false).await;

    let view = host.set_enabled(WORKERS_KIND, true).await.unwrap();

    assert_eq!(view.state, PluginState::Failed);
    assert_eq!(view.failure.as_deref(), Some(SERVICE_UNAVAILABLE));
    assert_eq!(rename(&host).await, Err(plugin_off()));
    assert!(!offers_delegate_task(&harness, "FAILED").await);
}

#[tokio::test]
async fn workers_keeps_the_rules_of_the_sdk() {
    let problems = conformance(mannie_workers::mannie_plugin(), options(true)).await;

    assert_eq!(problems, Vec::<String>::new());
}
