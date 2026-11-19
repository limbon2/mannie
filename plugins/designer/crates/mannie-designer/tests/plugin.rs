//! The plugin as the host sees it: its manifest, its interface, the rules of a
//! project's name, the SDK's rules through the real host, and a project that
//! the host's dispatch creates and the plugin's channel brings.

use futures_util::{FutureExt as _, StreamExt as _};
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
        [
            "create_project",
            "place_viewport",
            "set_viewport_color",
            "read_viewport_document",
            "commit_viewport_color"
        ]
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
    let [Call::Open(scope, _hidden)] = calls.as_slice() else {
        panic!("unexpected calls: {calls:?}")
    };
    assert_eq!(scope, &GraphicsView::new(DESIGNER, id));
    let target = json!({"projectId": id, "lease": "lease-1"});
    hosted
        .invoke("read_viewport_document", target)
        .await
        .unwrap();

    let rect = json!({"x": 0, "y": 0, "width": 640, "height": 400});
    hosted
        .invoke(
            "place_viewport",
            json!({
                "projectId": id, "lease": "lease-1",
                "placement": {"bounds": rect, "clip": rect, "visible": true, "scaleFactor": 1}
            }),
        )
        .await
        .unwrap();

    let started = graphics.calls();
    let [
        Call::Draw(_, _, _),
        Call::Draw(_, _, initial),
        Call::Place(_, _, _),
    ] = started.as_slice()
    else {
        panic!("unexpected calls: {started:?}")
    };
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
    let [Call::Draw(_, _, scene), Call::Place(address, lease, placed)] = placements.as_slice()
    else {
        panic!("unexpected calls: {placements:?}")
    };
    assert_eq!(address, &scope);
    assert_eq!(lease, "lease-1");
    assert!(scene.valid());
    let rect = scene.rectangle;
    assert!((rect.x - 64.0 / 200.0).abs() < f64::EPSILON);
    assert!((rect.y - 64.0 / 100.0).abs() < f64::EPSILON);
    assert!((rect.x + rect.width - 1.0).abs() < f64::EPSILON);
    assert!((rect.y + rect.height - 1.0).abs() < f64::EPSILON);
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

#[tokio::test]
async fn old_revisions_and_closed_or_replaced_leases_cannot_commit() {
    let graphics = RecordingGraphics::default();
    let hosted = HostedPlugin::new(mannie_plugin(), graphics.options()).await;

    hosted.turn_on().await;

    let (first, scope, _) = open_viewport(&hosted, &graphics).await;
    let target = json!({"projectId": scope.project, "lease": "lease-1"});
    let document = hosted
        .invoke("read_viewport_document", target.clone())
        .await
        .unwrap();
    let edit = json!({
        "projectId": scope.project, "lease": "lease-1", "elementId": document["element"]["id"],
        "revision": document["revision"], "hue": 240
    });

    let saved = hosted
        .invoke("commit_viewport_color", edit.clone())
        .await
        .unwrap();
    assert_eq!(saved["revision"], 2);
    assert_eq!(saved["element"]["hue"], 240);
    let error = hosted
        .invoke("commit_viewport_color", edit.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code(), "designer.stale_revision");
    let read = hosted
        .invoke("read_viewport_document", target)
        .await
        .unwrap();
    assert_eq!(read, saved);

    let mut second = hosted
        .open_channel("follow_viewport", json!({"projectId": scope.project}))
        .await
        .unwrap();
    assert_eq!(
        second.next().await,
        Some(ChannelMessage::Item {
            item: json!({"kind": "ready", "lease": "lease-2"})
        })
    );
    let error = hosted
        .invoke("commit_viewport_color", edit.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code(), "designer.stale_lease");

    drop(first);

    let current = json!({"projectId": scope.project, "lease": "lease-2"});
    assert_eq!(
        hosted
            .invoke("read_viewport_document", current.clone())
            .await
            .unwrap(),
        saved
    );

    let other = hosted
        .invoke("create_project", json!({"name": "Other"}))
        .await
        .unwrap();
    let mut wrong_project = edit.clone();
    wrong_project["projectId"] = other["id"].clone();
    wrong_project["lease"] = json!("lease-2");
    assert_eq!(
        hosted
            .invoke("commit_viewport_color", wrong_project)
            .await
            .unwrap_err()
            .code(),
        "designer.stale_lease"
    );

    drop(second);
    assert_eq!(
        hosted
            .invoke("read_viewport_document", current)
            .await
            .unwrap_err()
            .code(),
        "designer.stale_lease"
    );
}

#[tokio::test]
async fn storage_failures_redraw_the_confirmed_fill_and_recover_on_the_same_lease() {
    let graphics = RecordingGraphics::default();
    let hosted = HostedPlugin::new(mannie_plugin(), graphics.options()).await;

    hosted.turn_on().await;

    let (_viewport, scope, _) = open_viewport(&hosted, &graphics).await;
    let target = json!({"projectId": scope.project, "lease": "lease-1"});
    let document = hosted
        .invoke("read_viewport_document", target.clone())
        .await
        .unwrap();
    let edit = json!({
        "projectId": scope.project, "lease": "lease-1", "elementId": document["element"]["id"],
        "revision": document["revision"], "hue": 120
    });
    let preview = json!({"projectId": scope.project, "lease": "lease-1", "hue": 120});
    hosted.invoke("set_viewport_color", preview).await.unwrap();
    hosted
        .core
        .store
        .execute_unlisted(
            "CREATE TRIGGER refuse_fill BEFORE UPDATE OF hue ON designer_elements
         BEGIN SELECT RAISE(ABORT, 'refuse the test fill'); END",
        )
        .await
        .unwrap();

    let error = hosted
        .invoke("commit_viewport_color", edit.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code(), "designer.storage_failed");
    let calls = graphics.calls();
    let Some(Call::Draw(_, _, confirmed)) = calls.last() else {
        panic!("no rollback frame: {calls:?}");
    };
    assert_eq!(confirmed.rectangle.color, GraphicsColor::new(255, 0, 0));
    assert_eq!(
        hosted
            .invoke("read_viewport_document", target.clone())
            .await
            .unwrap(),
        document
    );

    hosted
        .core
        .store
        .execute_unlisted("DROP TRIGGER refuse_fill")
        .await
        .unwrap();

    let saved = hosted.invoke("commit_viewport_color", edit).await.unwrap();
    assert_eq!(saved["element"]["hue"], 120);
    hosted
        .core
        .store
        .execute_unlisted("ALTER TABLE designer_elements RENAME TO hidden_test_elements")
        .await
        .unwrap();

    let error = hosted
        .invoke("read_viewport_document", target.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code(), "designer.storage_failed");
    let calls = graphics.calls();
    let [.., Call::Draw(_, _, fallback), Call::Place(_, _, placement)] = calls.as_slice() else {
        panic!("no confirmed frame: {calls:?}");
    };
    assert!(placement.visible);
    assert_eq!(fallback.rectangle.color, GraphicsColor::new(0, 255, 0));
    hosted
        .core
        .store
        .execute_unlisted("ALTER TABLE hidden_test_elements RENAME TO designer_elements")
        .await
        .unwrap();
    assert_eq!(
        hosted
            .invoke("read_viewport_document", target)
            .await
            .unwrap(),
        saved
    );
}

#[tokio::test]
async fn concurrent_first_polls_cannot_deliver_an_old_ready_over_the_new_surface() {
    let graphics = RecordingGraphics::default();
    let hosted = HostedPlugin::new(mannie_plugin(), graphics.options()).await;

    hosted.turn_on().await;

    let project = hosted
        .invoke("create_project", json!({"name": "Studio"}))
        .await
        .unwrap();
    let scope = json!({"projectId": project["id"]});
    let resume = graphics.delay_next_ready();
    let mut first = hosted
        .open_channel("follow_viewport", scope.clone())
        .await
        .unwrap();
    let mut second = hosted.open_channel("follow_viewport", scope).await.unwrap();

    assert!(first.next().now_or_never().is_none());
    assert!(second.next().now_or_never().is_none());
    resume.send(()).unwrap();
    assert_eq!(
        first.next().await,
        Some(ChannelMessage::Item {
            item: json!({"kind": "ready", "lease": "lease-1"})
        })
    );
    assert_eq!(
        second.next().await,
        Some(ChannelMessage::Item {
            item: json!({"kind": "ready", "lease": "lease-2"})
        })
    );

    drop(first);

    let target = json!({"projectId": project["id"], "lease": "lease-2"});
    let document = hosted
        .invoke("read_viewport_document", target)
        .await
        .unwrap();
    assert_eq!(document["revision"], 1);
}
