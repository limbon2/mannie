use super::*;

use mannie_runtime::testing::Harness;
use mannie_store::Durability;
use mannie_store::{Change, NewConversation, TaskOrigin};
use mannie_types::file_links::OpenFileLink;
use mannie_types::ids::{ConversationId, ProjectId};

#[test]
fn files_directories_unicode_and_source_locations_resolve() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("source é file.rs");
    std::fs::write(&file, "content").unwrap();
    let absolute = file.to_str().unwrap();
    let plain = resolve_path(absolute, None).unwrap();
    assert_eq!(plain.kind, FileLinkKind::File);
    for suffix in [":42", ":42:7", "#L42"] {
        assert_eq!(
            resolve_path(&format!("{absolute}{suffix}"), None).unwrap(),
            plain
        );
    }
    assert_eq!(
        resolve_path("source é file.rs", Some(directory.path())).unwrap(),
        plain
    );
    let url = url::Url::from_file_path(&file).unwrap();
    assert_eq!(resolve_path(url.as_str(), None).unwrap(), plain);
    let mut localhost = url.clone();
    localhost.set_host(Some("localhost")).unwrap();
    assert_eq!(resolve_path(localhost.as_str(), None).unwrap(), plain);
    assert_eq!(resolve_path(&format!("{url}#L42"), None).unwrap(), plain);
    assert_eq!(
        resolve_path(directory.path().to_str().unwrap(), None)
            .unwrap()
            .kind,
        FileLinkKind::Directory
    );
}

#[test]
fn literal_names_take_precedence_and_native_percent_escapes_stay_literal() {
    let directory = tempfile::tempdir().unwrap();
    for file in ["main.rs", "main.rs#L42", "space%20name.rs", "space name.rs"] {
        std::fs::write(directory.path().join(file), "content").unwrap();
    }
    for file in ["main.rs#L42", "space%20name.rs"] {
        let resolved = resolve_path(file, Some(directory.path())).unwrap();
        assert!(resolved.path.ends_with(file));
    }
    let encoded = url::Url::from_file_path(directory.path().join("space name.rs")).unwrap();
    assert!(
        resolve_path(encoded.as_str(), None)
            .unwrap()
            .path
            .ends_with("space name.rs")
    );
    #[cfg(not(windows))]
    {
        std::fs::write(directory.path().join("main.rs:42"), "literal").unwrap();
        assert!(
            resolve_path("main.rs:42", Some(directory.path()))
                .unwrap()
                .path
                .ends_with("main.rs:42")
        );
    }
}

#[test]
fn invalid_remote_and_ambiguous_paths_are_refused_before_opening() {
    for path in [
        "",
        "a\0b",
        "a\nb",
        "javascript:alert(1)",
        "javascript:123",
        "mailto:123",
        "https://example.com/file",
        "file:relative",
        "file:///tmp/file?query=1",
    ] {
        assert!(resolve_path(path, None).is_err(), "{path:?}");
    }
    for path in [
        r"\\host\share\file",
        "//host/share/file",
        "file://host/share/file",
        "file://host/C:/file",
        "file:////host/share/file",
        r"\\?\C:\file",
        r"\\.\NUL",
    ] {
        assert_eq!(
            resolve_path(path, None),
            Err(FileLinkError::NotLocal),
            "{path:?}"
        );
    }
    assert_eq!(
        resolve_path("relative.rs", None),
        Err(FileLinkError::NoProject)
    );
    assert_eq!(
        resolve_path(&"a".repeat(MAX_URL_BYTES + 1), None),
        Err(FileLinkError::InvalidPath)
    );
    assert!(matches!(
        serde_json::from_value::<OpenFileLink>(serde_json::json!({
            "link": { "conversationId": null, "path": "file.rs" }, "target": "powershell"
        }))
        .unwrap()
        .target,
        FileLinkTarget::Unsupported
    ));
}

#[cfg(windows)]
#[test]
fn windows_devices_streams_and_drive_relative_paths_are_refused() {
    let directory = tempfile::tempdir().unwrap();
    for path in [
        "NUL",
        "CON.txt",
        "COM1",
        "COM\u{b9}",
        "lpt\u{b2}.txt",
        "subdir\\Com\u{b3}",
        "file:///C:/LPT%C2%B9",
        "PRN",
        "CONIN$",
        "C:relative",
        "file:///C:",
        "file:///C%3a",
        "file:///C:#L3",
        r"\relative",
    ] {
        assert_eq!(
            resolve_path(path, Some(directory.path())),
            Err(FileLinkError::InvalidPath),
            "{path}"
        );
    }
    std::fs::write(directory.path().join("file.txt"), "content").unwrap();
    assert_eq!(
        resolve_path("file.txt:stream", Some(directory.path())),
        Err(FileLinkError::InvalidPath)
    );
}

#[test]
fn paths_are_rechecked_after_the_file_was_removed() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("removed.rs");
    std::fs::write(&path, "content").unwrap();
    resolve_path(path.to_str().unwrap(), None).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        resolve_path(path.to_str().unwrap(), None),
        Err(FileLinkError::NotFound)
    );
}

#[cfg(unix)]
#[test]
fn symlinks_resolve_and_special_files_are_rejected() {
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("target"), "content").unwrap();
    symlink("target", directory.path().join("link")).unwrap();
    assert!(
        resolve_path("link", Some(directory.path()))
            .unwrap()
            .path
            .ends_with("target")
    );
    symlink("absent", directory.path().join("broken")).unwrap();
    assert_eq!(
        resolve_path("broken", Some(directory.path())),
        Err(FileLinkError::NotFound)
    );
    let _socket = UnixListener::bind(directory.path().join("socket")).unwrap();
    assert_eq!(
        resolve_path("socket", Some(directory.path())),
        Err(FileLinkError::UnsupportedKind)
    );
}

#[tokio::test]
async fn each_conversation_and_child_uses_its_own_persisted_project() {
    let harness = Harness::start().await;
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first");
    let second = directory.path().join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    std::fs::write(first.join("main.rs"), "first").unwrap();
    std::fs::write(second.join("main.rs"), "second").unwrap();
    let parent = ConversationId::parse("0199e0f2-0000-7000-8000-000000000091").unwrap();
    let other = ConversationId::parse("0199e0f2-0000-7000-8000-000000000092").unwrap();
    let child = ConversationId::parse("0199e0f2-0000-7000-8000-000000000093").unwrap();
    let general = ConversationId::parse("0199e0f2-0000-7000-8000-000000000094").unwrap();
    let project = ProjectId::parse("0199e0f2-0000-7000-8000-000000000095").unwrap();
    let other_project = ProjectId::parse("0199e0f2-0000-7000-8000-000000000096").unwrap();
    harness
        .store
        .add_project(&project, first.to_str().unwrap())
        .await
        .unwrap();
    harness
        .store
        .add_project(&other_project, second.to_str().unwrap())
        .await
        .unwrap();
    for (id, project_id, task) in [
        (&parent, Some(project), None),
        (&other, Some(other_project), None),
        (
            &child,
            None,
            Some(TaskOrigin {
                parent_id: parent.clone(),
                number: 1,
                agent: "general".to_owned(),
                executor: None,
            }),
        ),
        (&general, None, None),
    ] {
        harness
            .store
            .apply(
                id,
                vec![Change::CreateConversation {
                    conversation: NewConversation {
                        id: id.clone(),
                        title: "File navigation".to_owned(),
                        profile_id: None,
                        project_id,
                        task,
                        home: None,
                        agent_key: None,
                    },
                }],
                Durability::Full,
            )
            .await
            .unwrap();
    }
    for (id, base) in [(&parent, &first), (&other, &second), (&child, &first)] {
        let result = resolve(
            Some(&harness.store),
            FileLinkRef {
                conversation_id: Some(id.clone()),
                path: "main.rs".to_owned(),
            },
        )
        .await
        .unwrap();
        assert_eq!(result, resolve_path("main.rs", Some(base)).unwrap());
    }
    assert_eq!(
        resolve(
            Some(&harness.store),
            FileLinkRef {
                conversation_id: Some(general),
                path: "main.rs".to_owned(),
            }
        )
        .await,
        Err(FileLinkError::NoProject)
    );
    let missing = ConversationId::parse("0199e0f2-0000-7000-8000-000000000099").unwrap();
    assert_eq!(
        resolve(
            Some(&harness.store),
            FileLinkRef {
                conversation_id: Some(missing),
                path: first.to_string_lossy().into_owned(),
            }
        )
        .await,
        Err(FileLinkError::ConversationMissing)
    );
    harness.runtime.shutdown().await;
}

/// Run explicitly on a desktop; the caller owns and removes the fixture.
#[cfg(not(feature = "validation"))]
#[test]
#[ignore = "opens the native file manager; requires MANNIE_FILE_LINK_TEST_PATH"]
fn native_file_manager_opens_a_directory_and_reveals_a_file() {
    let path = std::env::var("MANNIE_FILE_LINK_TEST_PATH").expect("an explicit fixture directory");
    let directory = resolve_path(&path, None).unwrap();
    assert_eq!(directory.kind, FileLinkKind::Directory);
    launch(&directory).unwrap();
    let file = resolve_path("native.txt", Some(Path::new(&path))).unwrap();
    assert_eq!(file.kind, FileLinkKind::File);
    launch(&file).unwrap();
}
