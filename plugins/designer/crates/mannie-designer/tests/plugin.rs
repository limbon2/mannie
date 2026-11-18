//! The plugin as the host sees it: its manifest, its interface, the rules of a
//! project's name, the SDK's rules through the real host, and a project that
//! the host's dispatch creates and the plugin's channel brings.

use futures_util::StreamExt as _;
use mannie_designer::commands::interface;
use mannie_designer::plugin::DESIGNER;
use mannie_designer::{PLUGIN_MANIFEST, mannie_plugin};
use mannie_plugin_harness::conformance::conformance;
use mannie_plugin_harness::host::{CallFailure, HostOptions, HostedPlugin};
use mannie_plugin_sdk::ipc::ChannelMessage;
use mannie_plugin_sdk::manifest::Manifest;
use serde_json::{Value, json};

#[test]
fn the_manifest_names_the_plugin_and_its_table() {
    let manifest = Manifest::parse(PLUGIN_MANIFEST).unwrap();

    assert_eq!(manifest.id.as_str(), DESIGNER);
    assert!(manifest.declares("store"));
}

#[test]
fn the_interface_lists_the_command_and_the_channel() {
    let declared = interface();
    let exported = mannie_plugin().plugin.interface();

    assert_eq!(declared.command_names(), ["create_project"]);
    assert_eq!(declared.channel_names(), ["follow_projects"]);
    assert_eq!(exported.command_names(), declared.command_names());
    assert_eq!(exported.channel_names(), declared.channel_names());
}

#[tokio::test]
async fn the_plugin_keeps_the_rules_of_the_sdk() {
    assert_eq!(
        conformance(mannie_plugin(), HostOptions::new()).await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn the_channel_sends_the_initial_list_and_then_the_created_project() {
    let hosted = HostedPlugin::new(mannie_plugin(), HostOptions::new()).await;

    hosted.turn_on().await;

    let mut projects = hosted
        .open_channel("follow_projects", Value::Null)
        .await
        .unwrap();

    assert_eq!(
        projects.next().await,
        Some(ChannelMessage::Item {
            item: json!({ "projects": [] })
        })
    );

    let created = hosted
        .invoke("create_project", json!({ "name": "Landing page" }))
        .await
        .unwrap();
    let followed = projects.next().await;

    assert_eq!(created["name"], "Landing page");
    assert_eq!(
        followed,
        Some(ChannelMessage::Item {
            item: json!({ "projects": [created] })
        })
    );
}

#[tokio::test]
async fn project_names_are_unique_ignoring_ascii_case() {
    let hosted = HostedPlugin::new(mannie_plugin(), HostOptions::new()).await;

    hosted.turn_on().await;
    hosted
        .invoke("create_project", json!({ "name": "Landing page" }))
        .await
        .unwrap();

    let again = hosted
        .invoke("create_project", json!({ "name": "landing PAGE" }))
        .await;
    let code = match again {
        Err(CallFailure::Plugin(failure)) => failure.code,
        other => panic!("not an error of the plugin: {other:?}"),
    };

    assert_eq!(code, "designer.name_taken");
}
