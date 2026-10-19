//! The plugin as the host sees it: its manifest, its interface, its table,
//! the rules of a note, the SDK's rules through the real host, and a note
//! that the host's dispatch adds and the plugin's channel brings.

use futures_util::StreamExt as _;
use mannie_example::commands::{AddNote, MAX_NOTE_CHARACTERS, interface};
use mannie_example::plugin::{EXAMPLE, MODULE};
use mannie_example::{PLUGIN_MANIFEST, mannie_plugin};
use mannie_plugin_harness::conformance::conformance;
use mannie_plugin_harness::host::{HostOptions, HostedPlugin};
use mannie_plugin_sdk::ipc::{ChannelMessage, InputRefusal, Validate};
use mannie_plugin_sdk::manifest::Manifest;
use serde_json::{Value, json};

#[test]
fn the_manifest_parses_and_names_the_module_of_the_plugins_tables() {
    let manifest = Manifest::parse(PLUGIN_MANIFEST).unwrap();

    assert_eq!(manifest.id.as_str(), EXAMPLE);
    assert!(manifest.declares("store"));
    assert_eq!(MODULE.name, manifest.id.prefix());
    for migration in MODULE.migrations {
        assert!(
            migration.sql().contains("example_notes"),
            "{}",
            migration.description()
        );
    }
}

#[test]
fn the_interface_lists_the_command_and_the_channel() {
    let declared = interface();
    let exported = mannie_plugin().plugin.interface();

    assert_eq!(declared.command_names(), ["add_note"]);
    assert_eq!(declared.channel_names(), ["follow_notes"]);
    assert_eq!(exported.command_names(), declared.command_names());
}

#[test]
fn a_note_is_text_that_is_neither_blank_nor_too_long_nor_controlled() {
    let note = |text: &str| AddNote {
        text: text.to_owned(),
    };
    let longest = "a".repeat(MAX_NOTE_CHARACTERS as usize);
    let too_long = "a".repeat(MAX_NOTE_CHARACTERS as usize + 1);

    assert_eq!(note("Buy milk\nand bread").validate(), Ok(()));
    assert_eq!(note(&longest).validate(), Ok(()));
    for refused in ["", "   ", "a\tb", too_long.as_str()] {
        assert_eq!(
            note(refused).validate(),
            Err(InputRefusal::new("text")),
            "{refused:?}"
        );
    }
}

#[tokio::test]
async fn the_plugin_keeps_the_rules_of_the_sdk() {
    assert_eq!(
        conformance(mannie_plugin(), HostOptions::new()).await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn a_note_that_the_command_keeps_reaches_the_channel() {
    let hosted = HostedPlugin::new(mannie_plugin(), HostOptions::new()).await;
    hosted.turn_on().await;
    let mut notes = hosted
        .open_channel("follow_notes", Value::Null)
        .await
        .unwrap();

    let added = hosted
        .invoke("add_note", json!({ "text": "Buy milk" }))
        .await
        .unwrap();
    let followed = notes.next().await;

    assert_eq!(added["text"], "Buy milk");
    assert_eq!(followed, Some(ChannelMessage::Item { item: added }));
}
