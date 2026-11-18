//! The plugin as the host sees it: its manifest, its interface, the rules of a
//! project's name, the SDK's rules through the real host, and a project that
//! the host's dispatch creates and the plugin's channel brings.

use futures_util::StreamExt as _;
use mannie_designer::commands::interface;
use mannie_designer::plugin::DESIGNER;
use mannie_designer::{PLUGIN_MANIFEST, mannie_plugin};
use mannie_plugin_harness::conformance::conformance;
use mannie_plugin_harness::host::{CallFailure, HostedPlugin};
use mannie_plugin_sdk::ipc::{ChannelMessage, PluginStream};
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::services::graphics::{
    GraphicsColor, GraphicsError, GraphicsPlacement, GraphicsScene, GraphicsView,
};
use serde_json::{Value, json};

#[path = "support/graphics.rs"]
mod graphics;

use graphics::{Call, RecordingGraphics};

#[test]
fn the_manifest_names_the_plugin_store_and_graphics_services() {
    let manifest = Manifest::parse(PLUGIN_MANIFEST).unwrap();

    assert_eq!(manifest.id.as_str(), DESIGNER);
    assert!(manifest.declares("store"));
    assert!(manifest.declares("graphics"));
}

#[test]
fn the_interface_lists_the_commands_and_channels() {
    let declared = interface();
    let exported = mannie_plugin().plugin.interface();

    assert_eq!(
        declared.command_names(),
        ["create_project", "place_viewport", "set_viewport_color"]
    );
    assert_eq!(
        declared.channel_names(),
        ["follow_projects", "follow_viewport"]
    );
    assert_eq!(exported.command_names(), declared.command_names());
    assert_eq!(exported.channel_names(), declared.channel_names());
}

#[tokio::test]
async fn the_plugin_keeps_the_rules_of_the_sdk() {
    assert_eq!(
        conformance(mannie_plugin(), RecordingGraphics::default().options()).await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn the_channel_sends_the_initial_list_and_then_the_created_project() {
    let hosted = HostedPlugin::new(mannie_plugin(), RecordingGraphics::default().options()).await;

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
    let hosted = HostedPlugin::new(mannie_plugin(), RecordingGraphics::default().options()).await;

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

#[tokio::test]
async fn a_missing_project_cannot_open_a_viewport() {
    let graphics = RecordingGraphics::default();
    let hosted = HostedPlugin::new(mannie_plugin(), graphics.options()).await;

    hosted.turn_on().await;

    let missing = json!({"projectId": "0199e0f2-0000-7000-8000-000000000001"});
    let failure = hosted
        .open_channel("follow_viewport", missing)
        .await
        .map(|_| ())
        .unwrap_err();
    assert_eq!(failure.code(), "designer.project_missing");
    assert_eq!(graphics.calls(), Vec::new());
}

#[tokio::test]
async fn project_management_remains_available_when_graphics_are_unsupported() {
    let graphics = RecordingGraphics::unavailable(GraphicsError::UnsupportedPlatform);
    let hosted = HostedPlugin::new(mannie_plugin(), graphics.options()).await;

    hosted.turn_on().await;

    let project = hosted
        .invoke("create_project", json!({"name": "First"}))
        .await
        .unwrap();

    let failure = hosted
        .open_channel("follow_viewport", json!({"projectId": project["id"]}))
        .await
        .map(|_| ())
        .unwrap_err();
    assert_eq!(failure.code(), "designer.unsupported_platform");

    let second = hosted
        .invoke("create_project", json!({"name": "Second"}))
        .await
        .unwrap();
    assert_eq!(second["name"], "Second");
}

#[cfg(test)]
async fn open_viewport(
    hosted: &HostedPlugin,
    graphics: &RecordingGraphics,
) -> (
    PluginStream<ChannelMessage<Value>>,
    GraphicsView,
    GraphicsScene,
) {
    let project = hosted
        .invoke("create_project", json!({"name":"Studio"}))
        .await
        .unwrap();
    let id = project["id"].as_str().unwrap();
    let mut viewport = hosted
        .open_channel("follow_viewport", json!({"projectId":id}))
        .await
        .unwrap();
    assert_eq!(
        viewport.next().await,
        Some(ChannelMessage::Item {
            item: json!({"kind":"ready","lease":"lease-1"})
        })
    );
    let calls = graphics.calls();
    let [Call::Open(scope, initial)] = calls.as_slice() else {
        panic!("unexpected calls: {calls:?}")
    };
    assert_eq!(scope, &GraphicsView::new(DESIGNER, id));
    (viewport, scope.clone(), *initial)
}

#[tokio::test]
async fn a_project_draws_the_initial_red_rectangle_and_the_selected_hue() {
    let graphics = RecordingGraphics::default();
    let hosted = HostedPlugin::new(mannie_plugin(), graphics.options()).await;

    hosted.turn_on().await;

    let (_viewport, scope, initial) = open_viewport(&hosted, &graphics).await;
    assert!(initial.valid());
    assert_eq!(initial.rectangle.color, GraphicsColor::new(255, 0, 0));

    let color =
        |lease: &str, hue: u16| json!({"projectId": scope.project, "lease": lease, "hue": hue});
    assert_eq!(
        hosted
            .invoke("set_viewport_color", color("lease-1", 361))
            .await
            .unwrap_err()
            .code(),
        "invalid_input"
    );
    assert_eq!(
        hosted
            .invoke("set_viewport_color", color("old", 240))
            .await
            .unwrap_err()
            .code(),
        "designer.stale_lease"
    );
    hosted
        .invoke("set_viewport_color", color("lease-1", 240))
        .await
        .unwrap();

    let updated = graphics.calls();
    let [Call::Draw(address, lease, scene)] = updated.as_slice() else {
        panic!("unexpected calls: {updated:?}")
    };
    assert_eq!(address, &scope);
    assert_eq!(lease, "lease-1");
    assert_eq!(scene.rectangle.color, GraphicsColor::new(0, 0, 255));
    assert_eq!(scene.background, initial.background);
}

#[tokio::test]
async fn placement_targets_the_active_view_and_rejects_an_invalid_crop() {
    let graphics = RecordingGraphics::default();
    let hosted = HostedPlugin::new(mannie_plugin(), graphics.options()).await;

    hosted.turn_on().await;

    let (_viewport, scope, _scene) = open_viewport(&hosted, &graphics).await;
    let rect = json!({"x":0,"y":0,"width":200,"height":100});
    let placement = json!({"bounds":rect,"clip":rect,"scaleFactor":1.25,"visible":true});
    let input = json!({"projectId": scope.project, "lease": "lease-1", "placement": placement});
    hosted
        .invoke("place_viewport", input.clone())
        .await
        .unwrap();

    let placements = graphics.calls();
    let [Call::Place(address, lease, placed)] = placements.as_slice() else {
        panic!("unexpected calls: {placements:?}")
    };
    assert_eq!(address, &scope);
    assert_eq!(lease, "lease-1");
    assert_eq!(
        *placed,
        serde_json::from_value::<GraphicsPlacement>(placement).unwrap()
    );

    let mut malformed = input;
    malformed["placement"]["clip"]["width"] = json!(-1);
    assert_eq!(
        hosted
            .invoke("place_viewport", malformed)
            .await
            .unwrap_err()
            .code(),
        "invalid_input"
    );
    assert_eq!(graphics.calls(), Vec::new());
}
