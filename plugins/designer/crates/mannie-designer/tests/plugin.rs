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
    GraphicsColor, GraphicsError, GraphicsOverlay, GraphicsPlacement, GraphicsRectangle,
    GraphicsScene, GraphicsShape, GraphicsView,
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
            "commit_viewport_color",
            "set_viewport_selection"
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
    let [Call::Open(scope, hidden)] = calls.as_slice() else {
        panic!("unexpected calls: {calls:?}")
    };
    assert_eq!(scope, &GraphicsView::new(DESIGNER, id));
    assert!(hidden.shapes.is_empty() && hidden.overlays.is_empty());
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
    let [Call::Draw(_, _, initial), Call::Place(_, _, _)] = started.as_slice() else {
        panic!("unexpected calls: {started:?}")
    };
    (viewport, scope.clone(), initial.clone())
}

fn rectangle(scene: &GraphicsScene) -> &GraphicsRectangle {
    assert_eq!(scene.shapes.len(), 1, "expected one rectangle: {scene:?}");
    let GraphicsShape::Rectangle(rect) = &scene.shapes[0];
    rect
}

fn drawn_scene(graphics: &RecordingGraphics) -> GraphicsScene {
    let calls = graphics.calls();
    assert_eq!(calls.len(), 1, "expected one graphics call: {calls:?}");
    let scene = calls
        .into_iter()
        .find_map(|call| match call {
            Call::Draw(_, _, scene) => Some(scene),
            Call::Open(..) | Call::Place(..) => None,
        })
        .expect("the call should replace the scene");

    assert!(scene.valid());
    scene
}

#[tokio::test]
async fn a_project_draws_the_initial_red_rectangle_and_the_selected_hue() {
    let graphics = RecordingGraphics::default();
    let hosted = HostedPlugin::new(mannie_plugin(), graphics.options()).await;

    hosted.turn_on().await;

    let (_viewport, scope, initial) = open_viewport(&hosted, &graphics).await;
    assert!(initial.valid());
    assert_eq!(rectangle(&initial).color, GraphicsColor::new(255, 0, 0));
    assert_eq!(initial.overlays, Vec::new());

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
    assert_eq!(rectangle(scene).color, GraphicsColor::new(0, 0, 255));
    assert_eq!(scene.background, initial.background);
}

#[tokio::test]
async fn placement_targets_the_active_view_and_rejects_an_invalid_crop() {
    let graphics = RecordingGraphics::default();
    let hosted = HostedPlugin::new(mannie_plugin(), graphics.options()).await;

    hosted.turn_on().await;

    let (_viewport, scope, scene) = open_viewport(&hosted, &graphics).await;
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
    assert!(scene.valid());
    let rect = rectangle(&scene);
    assert_eq!(
        [rect.x, rect.y, rect.width, rect.height],
        [64.0, 64.0, 240.0, 160.0]
    );
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
async fn selection_and_fill_share_the_scene_and_only_a_fill_commit_changes_the_document() {
    let graphics = RecordingGraphics::default();
    let hosted = HostedPlugin::new(mannie_plugin(), graphics.options()).await;

    hosted.turn_on().await;

    let (_viewport, scope, _) = open_viewport(&hosted, &graphics).await;
    let target = json!({"projectId": scope.project, "lease": "lease-1"});
    let document = hosted
        .invoke("read_viewport_document", target.clone())
        .await
        .unwrap();
    graphics.calls();

    let mut selection = target.clone();
    selection["elementId"] = document["element"]["id"].clone();

    let preview = json!({"projectId": scope.project, "lease": "lease-1", "hue": 120});
    hosted.invoke("set_viewport_color", preview).await.unwrap();
    assert_eq!(drawn_scene(&graphics).overlays, Vec::new());

    graphics.fail_next_draw();

    let failure = hosted
        .invoke("set_viewport_selection", selection.clone())
        .await
        .unwrap_err();
    assert_eq!(failure.code(), "designer.render_failed");
    assert_eq!(graphics.calls(), Vec::new());

    hosted
        .invoke("set_viewport_selection", selection.clone())
        .await
        .unwrap();

    let selected = drawn_scene(&graphics);
    let fill = rectangle(&selected);
    assert_eq!(fill.color, GraphicsColor::new(0, 255, 0));
    let white = GraphicsColor::new(255, 255, 255);
    let outline = GraphicsRectangle::new(fill.x, fill.y, fill.width, fill.height, white);
    assert_eq!(
        selected.overlays,
        [GraphicsOverlay::RectangleOutline(outline)]
    );
    hosted
        .invoke("set_viewport_selection", selection.clone())
        .await
        .unwrap();
    assert_eq!(graphics.calls(), Vec::new());

    let mut clear = selection.clone();
    clear["elementId"] = Value::Null;
    hosted
        .invoke("set_viewport_selection", clear)
        .await
        .unwrap();

    let cleared = drawn_scene(&graphics);
    assert_eq!(cleared.shapes, selected.shapes);
    assert_eq!(cleared.overlays, Vec::new());
    hosted
        .invoke("set_viewport_selection", selection.clone())
        .await
        .unwrap();
    assert_eq!(drawn_scene(&graphics), selected);

    let read = hosted
        .invoke("read_viewport_document", target)
        .await
        .unwrap();
    assert_eq!(read, document);
    let frames = graphics.calls();
    let [Call::Draw(_, _, confirmed), Call::Place(_, _, _)] = frames.as_slice() else {
        panic!("unexpected read: {frames:?}");
    };
    assert_eq!(rectangle(confirmed).color, GraphicsColor::new(255, 0, 0));
    assert_eq!(confirmed.overlays, selected.overlays);

    let mut edit = selection;
    edit["revision"] = document["revision"].clone();
    edit["hue"] = json!(240);

    let saved = hosted.invoke("commit_viewport_color", edit).await.unwrap();
    let scene = drawn_scene(&graphics);
    assert_eq!(scene.overlays, selected.overlays);
    assert_eq!(rectangle(&scene).color, GraphicsColor::new(0, 0, 255));
    assert_eq!(saved["revision"], 2);
}

#[tokio::test]
async fn selection_requires_the_current_element_project_and_lease() {
    let graphics = RecordingGraphics::default();
    let hosted = HostedPlugin::new(mannie_plugin(), graphics.options()).await;

    hosted.turn_on().await;

    let (_first, scope, _) = open_viewport(&hosted, &graphics).await;
    let target = json!({"projectId": scope.project, "lease": "lease-1"});
    let document = hosted
        .invoke("read_viewport_document", target.clone())
        .await
        .unwrap();
    graphics.calls();

    let mut selection = target;
    selection["elementId"] = document["element"]["id"].clone();
    for (field, value, code) in [
        ("elementId", "bad-id", "invalid_input"),
        (
            "elementId",
            "0199e0f2-0000-7000-8000-000000000001",
            "designer.stale_revision",
        ),
        (
            "projectId",
            "0199e0f2-0000-7000-8000-000000000001",
            "designer.stale_lease",
        ),
        ("lease", "old-lease", "designer.stale_lease"),
    ] {
        let mut wrong = selection.clone();
        wrong[field] = json!(value);

        let refused = hosted
            .invoke("set_viewport_selection", wrong)
            .await
            .unwrap_err();
        assert_eq!(refused.code(), code);
    }
    assert_eq!(graphics.calls(), Vec::new());
    hosted
        .invoke("set_viewport_selection", selection.clone())
        .await
        .unwrap();
    assert_eq!(drawn_scene(&graphics).overlays.len(), 1);

    let mut second = hosted
        .open_channel("follow_viewport", json!({"projectId": scope.project}))
        .await
        .unwrap();
    assert!(second.next().await.is_some());
    graphics.calls();

    let expired = hosted
        .invoke("set_viewport_selection", selection.clone())
        .await
        .unwrap_err();
    assert_eq!(expired.code(), "designer.stale_lease");
    selection["lease"] = json!("lease-2");

    let unread = hosted
        .invoke("set_viewport_selection", selection)
        .await
        .unwrap_err();
    assert_eq!(unread.code(), "designer.stale_revision");

    let fresh = json!({"projectId": scope.project, "lease": "lease-2"});
    let read = hosted
        .invoke("read_viewport_document", fresh)
        .await
        .unwrap();
    assert_eq!(read, document);
    assert_eq!(drawn_scene(&graphics).overlays, Vec::new());
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
    assert_eq!(rectangle(confirmed).color, GraphicsColor::new(255, 0, 0));
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
    assert_eq!(rectangle(fallback).color, GraphicsColor::new(0, 255, 0));
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
