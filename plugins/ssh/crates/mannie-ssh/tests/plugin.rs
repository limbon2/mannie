//! The plugin as the host sees it: its manifest, its interface, its table,
//! the SDK's rules through the real host, and the owner's commands against
//! an SSH server in the test's process (plugins/ssh/docs/README.md, hosts).

use std::sync::Arc;

use futures_util::StreamExt as _;
use mannie_plugin_harness::conformance::conformance;
use mannie_plugin_harness::host::{HostOptions, HostedPlugin};
use mannie_plugin_sdk::ipc::ChannelMessage;
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::testing::FakeSecrets;
use mannie_ssh::commands::interface;
use mannie_ssh::{MODULE, PLUGIN_MANIFEST, SSH, mannie_plugin};
use mannie_ssh_testing::{Login, Options, PASSWORD, TestServer, USERNAME};
use serde_json::{Value, json};

fn options(secrets: &FakeSecrets) -> HostOptions {
    HostOptions::new().with_provider(Arc::new(secrets.clone()))
}

/// The plugin, on, with `secrets` as the credential store.
async fn hosted(secrets: &FakeSecrets) -> HostedPlugin {
    let hosted = HostedPlugin::new(mannie_plugin(), options(secrets)).await;
    hosted.turn_on().await;
    hosted
}

async fn server(options: Options) -> TestServer {
    TestServer::start(options)
        .await
        .expect("the test server starts")
}

/// The input of `add_host` for a host of `server`.
fn host(name: &str, port: u16, password: &str) -> Value {
    json!({
        "name": name,
        "address": "127.0.0.1",
        "port": port,
        "username": USERNAME,
        "password": password,
        "note": "The test server.",
    })
}

/// Adds a host of `server` and returns its view.
async fn add(hosted: &HostedPlugin, name: &str, server: &TestServer) -> Value {
    hosted
        .invoke("add_host", host(name, server.port(), PASSWORD))
        .await
        .expect("the host is added")
}

/// Adds a host of `server`, scans its key and trusts it.
async fn add_trusted(hosted: &HostedPlugin, name: &str, server: &TestServer) -> Value {
    let added = add(hosted, name, server).await;
    let id = &added["id"];
    let scan = hosted
        .invoke("scan_host_key", json!({ "hostId": id }))
        .await
        .expect("the host's key is scanned");
    hosted
        .invoke(
            "trust_host_key",
            json!({ "hostId": id, "fingerprint": scan["key"]["fingerprint"] }),
        )
        .await
        .expect("the scanned key is trusted")
}

/// The code of a failed call, such as `ssh.auth_failed`.
async fn failure(hosted: &HostedPlugin, command: &str, input: Value) -> &'static str {
    hosted
        .invoke(command, input)
        .await
        .expect_err("the call fails")
        .code()
}

#[test]
fn the_manifest_parses_and_names_the_module_of_the_plugins_tables() {
    let manifest = Manifest::parse(PLUGIN_MANIFEST).unwrap();

    assert_eq!(manifest.id.as_str(), SSH);
    assert!(manifest.declares("store"));
    assert!(manifest.declares("secrets"));
    assert_eq!(manifest.secrets.keyed, ["host"]);
    assert_eq!(MODULE.name, manifest.id.prefix());
    for migration in MODULE.migrations {
        assert!(
            migration.sql().contains("ssh_hosts"),
            "{}",
            migration.description()
        );
    }
}

#[test]
fn the_interface_lists_the_commands_and_the_channel() {
    let declared = interface();
    let exported = mannie_plugin().plugin.interface();

    assert_eq!(
        declared.command_names(),
        [
            "add_host",
            "update_host",
            "remove_host",
            "scan_host_key",
            "trust_host_key",
            "test_host"
        ]
    );
    assert_eq!(declared.channel_names(), ["follow_hosts"]);
    assert_eq!(exported.command_names(), declared.command_names());
}

#[tokio::test]
async fn the_plugin_keeps_the_rules_of_the_sdk() {
    let secrets = FakeSecrets::new();

    assert_eq!(
        conformance(mannie_plugin(), options(&secrets)).await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn a_host_keeps_its_password_in_the_credential_store_and_never_shows_it() {
    let secrets = FakeSecrets::new();
    let hosted = hosted(&secrets).await;
    let server = server(Options::default()).await;

    let added = add(&hosted, "build-box", &server).await;

    let id = added["id"].as_str().unwrap();
    assert_eq!(added["name"], "build-box");
    assert_eq!(
        added["endpoint"],
        format!("{USERNAME}@127.0.0.1:{}", server.port())
    );
    assert_eq!(added["passwordSet"], true);
    assert_eq!(added["status"], "key_not_checked");
    assert_eq!(added["key"], Value::Null);
    assert_eq!(
        secrets.stored(&format!("ssh/host/{id}")).unwrap().expose(),
        PASSWORD
    );
    assert!(!added.to_string().contains(PASSWORD));
    // Adding a host connects to nothing.
    assert_eq!(server.auth_attempts(), 0);
}

#[tokio::test]
async fn a_name_belongs_to_one_host_without_regard_to_case() {
    let secrets = FakeSecrets::new();
    let hosted = hosted(&secrets).await;
    let server = server(Options::default()).await;
    add(&hosted, "build-box", &server).await;

    let taken = failure(
        &hosted,
        "add_host",
        host("Build-Box", server.port(), PASSWORD),
    )
    .await;

    assert_eq!(taken, "ssh.name_taken");
}

#[tokio::test]
async fn input_that_breaks_a_rule_is_refused_before_the_plugin_sees_it() {
    let secrets = FakeSecrets::new();
    let hosted = hosted(&secrets).await;

    let mut bad_address = host("box", 22, PASSWORD);
    bad_address["address"] = json!("ssh://box");
    let mut no_port = host("box", 22, PASSWORD);
    no_port["port"] = json!(0);

    assert_eq!(
        failure(&hosted, "add_host", bad_address).await,
        "invalid_input"
    );
    assert_eq!(failure(&hosted, "add_host", no_port).await, "invalid_input");
}

#[tokio::test]
async fn a_scan_reads_the_key_without_logging_in_and_trusting_it_makes_the_host_ready() {
    let secrets = FakeSecrets::new();
    let hosted = hosted(&secrets).await;
    let server = server(Options::default()).await;
    let added = add(&hosted, "build-box", &server).await;
    let id = &added["id"];

    let scan = hosted
        .invoke("scan_host_key", json!({ "hostId": id }))
        .await
        .unwrap();
    let attempts_after_scan = server.auth_attempts();
    let trusted = hosted
        .invoke(
            "trust_host_key",
            json!({ "hostId": id, "fingerprint": scan["key"]["fingerprint"] }),
        )
        .await
        .unwrap();

    assert_eq!(scan["key"]["fingerprint"], server.fingerprint());
    assert_eq!(scan["key"]["algorithm"], "ssh-ed25519");
    assert_eq!(scan["comparison"], "not_pinned");
    assert_eq!(attempts_after_scan, 0);
    assert_eq!(trusted["status"], "ready");
    assert_eq!(trusted["key"]["fingerprint"], server.fingerprint());
    assert_eq!(trusted["seenKey"], Value::Null);
}

#[tokio::test]
async fn trusting_a_key_that_the_last_scan_did_not_see_pins_nothing() {
    let secrets = FakeSecrets::new();
    let hosted = hosted(&secrets).await;
    let server = server(Options::default()).await;
    let added = add(&hosted, "build-box", &server).await;
    let id = &added["id"];

    let unscanned = failure(
        &hosted,
        "trust_host_key",
        json!({ "hostId": id, "fingerprint": server.fingerprint() }),
    )
    .await;
    hosted
        .invoke("scan_host_key", json!({ "hostId": id }))
        .await
        .unwrap();
    let other = failure(
        &hosted,
        "trust_host_key",
        json!({ "hostId": id, "fingerprint": "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA" }),
    )
    .await;

    assert_eq!(unscanned, "ssh.scan_outdated");
    assert_eq!(other, "ssh.scan_outdated");
}

#[tokio::test]
async fn a_test_login_takes_the_password_and_refuses_a_wrong_one() {
    let secrets = FakeSecrets::new();
    let hosted = hosted(&secrets).await;
    let server = server(Options::default()).await;
    let trusted = add_trusted(&hosted, "build-box", &server).await;
    let id = trusted["id"].clone();

    let tested = hosted.invoke("test_host", json!({ "hostId": id })).await;
    let mut wrong = host("build-box", server.port(), "not the password");
    wrong["hostId"] = id.clone();
    hosted.invoke("update_host", wrong).await.unwrap();
    let refused = failure(&hosted, "test_host", json!({ "hostId": id })).await;

    assert_eq!(tested.unwrap(), Value::Null);
    assert_eq!(refused, "ssh.auth_failed");
    assert!(server.commands().is_empty(), "{:?}", server.commands());
}

#[tokio::test]
async fn keyboard_interactive_login_takes_the_password_in_its_one_hidden_prompt() {
    let secrets = FakeSecrets::new();
    let hosted = hosted(&secrets).await;
    let server = server(Options {
        login: Login::KeyboardInteractive,
        ..Options::default()
    })
    .await;
    let trusted = add_trusted(&hosted, "freebsd", &server).await;

    let tested = hosted
        .invoke("test_host", json!({ "hostId": trusted["id"] }))
        .await;

    assert_eq!(tested.unwrap(), Value::Null);
}

#[tokio::test]
async fn a_changed_key_stops_the_login_before_the_password_is_sent() {
    let secrets = FakeSecrets::new();
    let hosted = hosted(&secrets).await;
    let server = server(Options::default()).await;
    let trusted = add_trusted(&hosted, "build-box", &server).await;
    let id = trusted["id"].clone();
    server.replace_key().unwrap();

    let refused = failure(&hosted, "test_host", json!({ "hostId": id })).await;
    let scan = hosted
        .invoke("scan_host_key", json!({ "hostId": id }))
        .await
        .unwrap();

    assert_eq!(refused, "ssh.host_key_changed");
    assert_eq!(server.auth_attempts(), 0);
    assert_eq!(scan["comparison"], "changed");
    assert_eq!(scan["key"]["fingerprint"], server.fingerprint());
    // The host now shows both keys, and runs nothing until the owner trusts
    // the new one.
    let mut hosts = hosted
        .open_channel("follow_hosts", Value::Null)
        .await
        .unwrap();
    let Some(ChannelMessage::Item { item }) = hosts.next().await else {
        panic!("the channel brings the hosts");
    };
    let [view]: [Value; 1] = serde_json::from_value(item["hosts"].clone()).unwrap();
    assert_eq!(view["status"], "key_changed");
    assert_eq!(view["seenKey"]["fingerprint"], server.fingerprint());
    assert_ne!(view["key"]["fingerprint"], server.fingerprint());
}

#[tokio::test]
async fn a_host_that_offers_only_rsa_keys_is_refused() {
    let secrets = FakeSecrets::new();
    let hosted = hosted(&secrets).await;
    let server = server(Options {
        rsa_only: true,
        ..Options::default()
    })
    .await;
    let added = add(&hosted, "old-box", &server).await;

    let refused = failure(&hosted, "scan_host_key", json!({ "hostId": added["id"] })).await;

    assert_eq!(refused, "ssh.host_key_unsupported");
    assert_eq!(server.auth_attempts(), 0);
}

#[tokio::test]
async fn a_host_where_nothing_listens_is_unreachable() {
    let secrets = FakeSecrets::new();
    let hosted = hosted(&secrets).await;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let added = hosted
        .invoke("add_host", host("gone", port, PASSWORD))
        .await
        .unwrap();

    let refused = failure(&hosted, "scan_host_key", json!({ "hostId": added["id"] })).await;

    assert_eq!(refused, "ssh.unreachable");
}

#[tokio::test]
async fn another_address_or_port_lets_go_of_the_pinned_key_and_another_user_keeps_it() {
    let secrets = FakeSecrets::new();
    let hosted = hosted(&secrets).await;
    let server = server(Options::default()).await;
    let trusted = add_trusted(&hosted, "build-box", &server).await;
    let mut renamed = host("builder", server.port(), PASSWORD);
    renamed["hostId"] = trusted["id"].clone();
    renamed["username"] = json!("root");
    renamed.as_object_mut().unwrap().remove("password");
    let mut moved = renamed.clone();
    moved["port"] = json!(2222);

    let kept = hosted.invoke("update_host", renamed).await.unwrap();
    let forgotten = hosted.invoke("update_host", moved).await.unwrap();

    assert_eq!(kept["name"], "builder");
    assert_eq!(kept["status"], "ready");
    assert_eq!(
        kept["endpoint"],
        format!("root@127.0.0.1:{}", server.port())
    );
    assert_eq!(forgotten["key"], Value::Null);
    assert_eq!(forgotten["status"], "key_not_checked");
    // A change without a password keeps the stored one.
    let id = trusted["id"].as_str().unwrap();
    assert_eq!(
        secrets.stored(&format!("ssh/host/{id}")).unwrap().expose(),
        PASSWORD
    );
}

#[tokio::test]
async fn removing_a_host_deletes_its_password_and_the_channel_follows() {
    let secrets = FakeSecrets::new();
    let hosted = hosted(&secrets).await;
    let server = server(Options::default()).await;
    let mut hosts = hosted
        .open_channel("follow_hosts", Value::Null)
        .await
        .unwrap();
    let first = hosts.next().await;
    let added = add(&hosted, "build-box", &server).await;
    let after_add = hosts.next().await;
    let id = added["id"].as_str().unwrap().to_owned();

    hosted
        .invoke("remove_host", json!({ "hostId": id }))
        .await
        .unwrap();
    let after_remove = hosts.next().await;
    let gone = failure(&hosted, "test_host", json!({ "hostId": id })).await;

    let count = |message: Option<ChannelMessage<Value>>| match message {
        Some(ChannelMessage::Item { item }) => item["hosts"].as_array().unwrap().len(),
        other => panic!("expected the hosts, got {other:?}"),
    };
    assert_eq!(count(first), 0);
    assert_eq!(count(after_add), 1);
    assert_eq!(count(after_remove), 0);
    assert!(secrets.stored(&format!("ssh/host/{id}")).is_none());
    assert_eq!(gone, "ssh.host_not_found");
}

#[tokio::test]
async fn a_host_whose_password_could_not_be_kept_is_not_registered() {
    let secrets = FakeSecrets::new();
    let hosted = hosted(&secrets).await;
    let server = server(Options::default()).await;
    secrets.set_unavailable(true);

    let refused = failure(
        &hosted,
        "add_host",
        host("build-box", server.port(), PASSWORD),
    )
    .await;
    secrets.set_unavailable(false);
    let added = add(&hosted, "build-box", &server).await;

    assert_eq!(refused, "ssh.credential_store_unavailable");
    assert_eq!(added["name"], "build-box");
}
