use mannie_plugin_harness::host::HostedPlugin;
use mannie_plugin_sdk::services::graphics::{
    GraphicsColor, GraphicsOverlay, GraphicsRectangle, GraphicsScene,
};
use serde_json::{Value, json};

use super::{RecordingGraphics, drawn_scene, mannie_plugin, open_viewport, rectangle};
use crate::graphics::Call;

async fn select_rectangle(
    hosted: &HostedPlugin,
    graphics: &RecordingGraphics,
    project_id: &str,
) -> (Value, Value, GraphicsScene) {
    let target = json!({"projectId": project_id, "lease": "lease-1"});
    let document = hosted
        .invoke("read_viewport_document", target.clone())
        .await
        .expect("the open viewport has a seeded rectangle");
    graphics.calls();

    let id = &document["element"]["id"];
    let selection = json!({"projectId": project_id, "lease": "lease-1", "elementId": id});
    hosted
        .invoke("set_viewport_selection", selection)
        .await
        .expect("the current lease can select its rectangle");

    let edit = json!({
        "projectId": project_id, "lease": "lease-1", "elementId": id, "revision": document["revision"],
        "x": -12.25, "y": 23.5, "width": 76.75, "height": 1.0
    });
    (target, edit, drawn_scene(graphics))
}

#[tokio::test]
async fn geometry_previews_the_fill_and_outline_and_commits_once() {
    let graphics = RecordingGraphics::default();
    let hosted = HostedPlugin::new(mannie_plugin(), graphics.options()).await;

    hosted.turn_on().await;

    let (_viewport, scope, _) = open_viewport(&hosted, &graphics).await;
    let (target, mut edit, initial) = select_rectangle(&hosted, &graphics, &scope.project).await;
    hosted
        .invoke("set_viewport_geometry", edit.clone())
        .await
        .unwrap();

    let preview = drawn_scene(&graphics);
    let expected = GraphicsRectangle::new(-12.25, 23.5, 76.75, 1.0, GraphicsColor::new(255, 0, 0));
    assert_eq!(*rectangle(&preview), expected);
    let mut outline = expected;
    outline.color = GraphicsColor::new(255, 255, 255);
    assert_eq!(
        preview.overlays,
        [GraphicsOverlay::RectangleOutline(outline)]
    );

    let read = hosted
        .invoke("read_viewport_document", target.clone())
        .await
        .unwrap();
    assert_eq!(read["revision"], 1);
    assert_eq!(read["element"]["x"], 64.0);
    let calls = graphics.calls();
    let [Call::Draw(_, _, restored), Call::Place(_, _, _)] = calls.as_slice() else {
        panic!("expected the confirmed scene after reading: {calls:?}");
    };
    assert_eq!(*restored, initial);

    graphics.fail_next_draw();

    let refusal = hosted
        .invoke("commit_viewport_geometry", edit.clone())
        .await
        .unwrap_err();
    assert_eq!(refusal.code(), "designer.render_failed");

    let saved = hosted
        .invoke("commit_viewport_geometry", edit.clone())
        .await
        .unwrap();
    assert_eq!(drawn_scene(&graphics), preview);
    assert_eq!(saved["revision"], 2);
    assert_eq!(saved["element"]["id"], edit["elementId"]);
    for field in ["x", "y", "width", "height"] {
        assert_eq!(saved["element"][field], edit[field]);
    }

    edit["revision"] = saved["revision"].clone();

    let unchanged = hosted
        .invoke("commit_viewport_geometry", edit)
        .await
        .unwrap();
    assert_eq!(unchanged, saved);
    assert_eq!(
        hosted
            .invoke("read_viewport_document", target)
            .await
            .unwrap(),
        saved
    );
}

#[tokio::test]
async fn hiding_the_surface_discards_geometry_but_keeps_the_confirmed_selection() {
    let graphics = RecordingGraphics::default();
    let hosted = HostedPlugin::new(mannie_plugin(), graphics.options()).await;

    hosted.turn_on().await;

    let (_viewport, scope, _) = open_viewport(&hosted, &graphics).await;
    let (target, edit, initial) = select_rectangle(&hosted, &graphics, &scope.project).await;
    hosted.invoke("set_viewport_geometry", edit).await.unwrap();
    assert_ne!(drawn_scene(&graphics), initial);

    let crop = json!({"x": 0, "y": 0, "width": 640, "height": 400});
    let mut hide = target.clone();
    hide["placement"] = json!({"bounds": crop, "clip": crop, "visible": false, "scaleFactor": 1});
    hosted.invoke("place_viewport", hide).await.unwrap();

    let calls = graphics.calls();
    let [Call::Place(_, _, placement), Call::Draw(_, _, restored)] = calls.as_slice() else {
        panic!("expected the hidden surface to discard its preview: {calls:?}");
    };
    assert!(!placement.visible);
    assert_eq!(*restored, initial);

    let read = hosted
        .invoke("read_viewport_document", target)
        .await
        .unwrap();
    assert_eq!(read["revision"], 1);
}

#[tokio::test]
async fn invalid_geometry_and_stale_targets_cannot_save_or_preview() {
    let graphics = RecordingGraphics::default();
    let hosted = HostedPlugin::new(mannie_plugin(), graphics.options()).await;

    hosted.turn_on().await;

    let (_viewport, scope, _) = open_viewport(&hosted, &graphics).await;
    let (target, edit, initial) = select_rectangle(&hosted, &graphics, &scope.project).await;

    for command in ["set_viewport_geometry", "commit_viewport_geometry"] {
        for (field, value, code) in [
            ("width", json!(0.999), "invalid_input"),
            ("height", json!(0), "invalid_input"),
            ("y", json!(32767.25), "invalid_input"),
            ("x", json!(-32768.25), "invalid_input"),
            ("elementId", json!("unknown"), "invalid_input"),
            ("lease", json!("old"), "designer.stale_lease"),
            (
                "projectId",
                json!("0199e0f2-0000-7000-8000-000000000001"),
                "designer.stale_lease",
            ),
        ] {
            let mut wrong = edit.clone();
            wrong[field] = value;

            let refusal = hosted.invoke(command, wrong).await.unwrap_err();
            assert_eq!(refusal.code(), code);
        }
        assert_eq!(graphics.calls(), Vec::new());

        for (field, value) in [
            ("revision", json!(0)),
            ("elementId", json!("0199e0f2-0000-7000-8000-000000000001")),
        ] {
            hosted
                .invoke("set_viewport_geometry", edit.clone())
                .await
                .unwrap();
            assert_ne!(drawn_scene(&graphics), initial);

            let mut stale = edit.clone();
            stale[field] = value;

            let refusal = hosted.invoke(command, stale).await.unwrap_err();
            assert_eq!(refusal.code(), "designer.stale_revision");
            assert_eq!(drawn_scene(&graphics), initial);

            let read = hosted
                .invoke("read_viewport_document", target.clone())
                .await
                .unwrap();
            assert_eq!(read["revision"], 1);
            graphics.calls();
        }
    }
}

#[tokio::test]
async fn a_failed_geometry_write_restores_the_confirmed_pixels_and_revision() {
    let graphics = RecordingGraphics::default();
    let hosted = HostedPlugin::new(mannie_plugin(), graphics.options()).await;

    hosted.turn_on().await;

    let (_viewport, scope, _) = open_viewport(&hosted, &graphics).await;
    let (target, edit, initial) = select_rectangle(&hosted, &graphics, &scope.project).await;
    hosted
        .invoke("set_viewport_geometry", edit.clone())
        .await
        .unwrap();

    let draft = drawn_scene(&graphics);

    hosted
        .core
        .store
        .execute_unlisted(
            "CREATE TRIGGER refuse_geometry BEFORE UPDATE OF width ON designer_elements
         BEGIN SELECT RAISE(ABORT, 'refuse the test geometry'); END",
        )
        .await
        .unwrap();

    let error = hosted
        .invoke("commit_viewport_geometry", edit.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code(), "designer.storage_failed");
    let calls = graphics.calls();
    let [Call::Draw(_, _, candidate), Call::Draw(_, _, restored)] = calls.as_slice() else {
        panic!("expected candidate then rollback: {calls:?}");
    };
    assert_eq!(*candidate, draft);
    assert_eq!(*restored, initial);

    let confirmed = hosted
        .invoke("read_viewport_document", target)
        .await
        .unwrap();
    assert_eq!(confirmed["revision"], 1);
    assert_eq!(confirmed["element"]["x"], 64.0);
    hosted
        .core
        .store
        .execute_unlisted("DROP TRIGGER refuse_geometry")
        .await
        .unwrap();

    let saved = hosted
        .invoke("commit_viewport_geometry", edit)
        .await
        .unwrap();
    assert_eq!(saved["revision"], 2);
}
