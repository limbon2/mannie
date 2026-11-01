//! Terminals on SSH hosts through the core's service `terminals` and its
//! consumer of the point `terminal-hosts` (ADR-0053, decision 3; plugins/
//! ssh/docs/README.md, terminals), against an SSH server in the test's
//! process: a terminal runs the login shell on a pseudo terminal of the
//! requested size, the login shell is listed once the first terminal read
//! it, the password never shows in the output even when the host's writes
//! cut it, a typed line reaches the shell whole, a host whose key changed
//! opens nothing and gets no password, and editing the host or turning SSH
//! off ends its terminals.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt as _;
use mannie_plugin_harness::host::{HostOptions, HostedPlugin};
use mannie_plugin_harness::runtime::{Harness, reply};
use mannie_plugin_harness::types::StartConversation;
use mannie_plugin_sdk::BoxStream;
use mannie_plugin_sdk::context::HostProfile;
use mannie_plugin_sdk::host::PluginBinding;
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::service::Service;
use mannie_plugin_sdk::services::terminals::{
    ExpectedHost, OfferedHost, ShellKind, Terminal, TerminalEnding, TerminalHostList,
    TerminalHostRef, TerminalHostState, TerminalOpenError, TerminalOutput, TerminalSize,
    TerminalSpec, Terminals, TerminalsApi,
};
use mannie_plugin_sdk::testing::FakeSecrets;
use mannie_ssh::{SSH, mannie_plugin};
use mannie_ssh_testing::{Options, PASSWORD, TestServer, USERNAME};
use serde_json::json;

/// The plugin that opens the terminals, in place of the terminal plugin.
const OPENER: &str = "terminal";

/// How long a test waits for something that should come at once.
const WAIT: Duration = Duration::from_secs(20);

/// The scripted shell's prompt.
const PROMPT: &str = "deploy$ ";

/// The plugin, on.
async fn hosted() -> HostedPlugin {
    let options = HostOptions::new().with_provider(Arc::new(FakeSecrets::new()));
    let hosted = HostedPlugin::new(mannie_plugin(), options).await;
    hosted.turn_on().await;
    hosted
}

/// The fields of a host of `server` named `name`, with `note`.
fn host_fields(name: &str, server: &TestServer, note: &str) -> serde_json::Value {
    json!({
        "name": name,
        "address": "127.0.0.1",
        "port": server.port(),
        "username": USERNAME,
        "password": PASSWORD,
        "note": note,
    })
}

/// Registers a host of `server` and trusts its key, as the owner does in
/// the settings. Returns the host's ID.
async fn register(hosted: &HostedPlugin, server: &TestServer) -> String {
    let added = hosted
        .invoke(
            "add_host",
            host_fields("test-box", server, "The test server."),
        )
        .await
        .expect("the host is added");
    let id = added["id"].as_str().expect("a host has an ID").to_owned();
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
        .expect("the scanned key is trusted");
    id
}

/// The core's service, bound to the plugin that opens terminals.
fn terminals(core: &Harness) -> Arc<dyn TerminalsApi> {
    let provider = core
        .runtime
        .service_providers()
        .into_iter()
        .find(|provider| provider.name() == Terminals::NAME)
        .expect("the core provides terminals");
    let id = PluginId::parse(OPENER).expect("the opener's ID is a plugin ID");
    let manifest: Manifest = serde_json::from_value(json!({
        "id": OPENER,
        "sdk": 1,
        "order": 10,
        "rust": { "entry": "mannie-terminal" },
        "services": { "terminals": 1 },
    }))
    .expect("the opener's manifest reads");
    provider
        .bind(&PluginBinding::new(
            &id,
            &manifest,
            HostProfile::Development,
        ))
        .expect("the core binds terminals to the opener")
        .get::<Terminals>()
        .expect("the binding is the service terminals")
}

/// A conversation whose first reply ended, which terminals belong to.
async fn conversation(core: &Harness) -> String {
    core.model.push_reply(reply("Done."));
    let started = core
        .runtime
        .start_conversation(StartConversation::new("Open a terminal."))
        .await
        .expect("the conversation starts");
    core.wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    started.conversation_id.to_string()
}

/// The host of the SSH plugin with this key, once a list of the core's
/// hosts shows it as `wanted` says.
async fn offered(
    lists: &mut BoxStream<'static, TerminalHostList>,
    key: &str,
    wanted: impl Fn(&OfferedHost) -> bool,
) -> OfferedHost {
    let found = tokio::time::timeout(WAIT, async {
        while let Some(list) = lists.next().await {
            let host = list
                .hosts
                .into_iter()
                .find(|host| host.plugin == SSH && host.key == key);
            if let Some(host) = host
                && wanted(&host)
            {
                return Some(host);
            }
        }
        None
    })
    .await
    .expect("the host is listed in time");
    found.expect("the lists go on until one shows the host")
}

fn size() -> TerminalSize {
    TerminalSize::new(100, 30).expect("the size is within the limits")
}

fn spec(conversation: &str, key: &str) -> TerminalSpec {
    let host = TerminalHostRef::Plugin {
        plugin: SSH.to_owned(),
        key: key.to_owned(),
    };
    TerminalSpec::new(conversation, host, size())
}

/// What a terminal printed so far, and how it ended.
#[derive(Default)]
struct Seen {
    text: String,
    ending: Option<TerminalEnding>,
}

/// Reads a terminal's output until `done` holds for what it printed, or it
/// ended.
async fn read_until(
    output: &mut BoxStream<'static, TerminalOutput>,
    seen: &mut Seen,
    done: impl Fn(&str) -> bool,
) {
    let went_on = tokio::time::timeout(WAIT, async {
        while !done(&seen.text) && seen.ending.is_none() {
            match output.next().await {
                Some(TerminalOutput::Bytes { bytes }) => {
                    seen.text.push_str(&String::from_utf8_lossy(&bytes));
                }
                Some(TerminalOutput::Ended { ending }) => seen.ending = Some(ending),
                Some(_other) => {}
                None => return false,
            }
        }
        true
    })
    .await
    .expect("the terminal printed it in time");
    assert!(went_on, "the output ended without an ending");
}

/// Types a line into the terminal, as the owner does.
async fn type_line(terminal: &dyn Terminal, line: &str) {
    terminal
        .write(format!("{line}\r").into_bytes())
        .await
        .expect("the terminal takes the line");
}

#[tokio::test]
async fn a_terminal_runs_the_login_shell_on_a_pseudo_terminal_and_lists_the_shell() {
    let hosted = hosted().await;
    let server = TestServer::start(Options::default()).await.unwrap();
    let key = register(&hosted, &server).await;
    let core = &hosted.core;
    let service = terminals(core);
    let conversation = conversation(core).await;
    let mut lists = service.hosts();
    let before = offered(&mut lists, &key, |host| {
        host.state == TerminalHostState::Ready
    })
    .await;

    let mut terminal = service
        .open(spec(&conversation, &key))
        .await
        .expect("the terminal opens");
    let mut output = terminal.output();
    let mut seen = Seen::default();
    read_until(&mut output, &mut seen, |text| text.contains(PROMPT)).await;
    let after = offered(&mut lists, &key, |host| host.shell.is_some()).await;
    type_line(terminal.as_ref(), "echo hello from the host").await;
    read_until(&mut output, &mut seen, |text| {
        text.contains("hello from the host\r\n")
    })
    .await;
    terminal
        .resize(TerminalSize::new(120, 40).unwrap())
        .await
        .expect("the terminal takes the size");
    type_line(terminal.as_ref(), "exit 3").await;
    read_until(&mut output, &mut seen, |_text| false).await;

    assert_eq!(
        before.endpoint,
        format!("{USERNAME}@127.0.0.1:{}", server.port())
    );
    assert_eq!(before.identity, server.fingerprint());
    assert_eq!(before.shell, None);
    assert_eq!(after.shell, Some(ShellKind::Bash));
    assert_eq!(seen.ending, Some(TerminalEnding::Exited { code: 3 }));
    assert_eq!(server.pty_sizes(), [(100, 30), (120, 40)]);
    assert_eq!(server.shell_lines(), ["echo hello from the host", "exit 3"]);
    assert_eq!(server.commands(), ["printf '%s' \"$SHELL\""]);
    core.runtime.shutdown().await;
}

#[tokio::test]
async fn the_password_never_shows_even_when_the_hosts_writes_cut_it() {
    let hosted = hosted().await;
    let server = TestServer::start(Options::default()).await.unwrap();
    let key = register(&hosted, &server).await;
    let core = &hosted.core;
    let service = terminals(core);
    let conversation = conversation(core).await;

    let mut terminal = service
        .open(spec(&conversation, &key))
        .await
        .expect("the terminal opens");
    let mut output = terminal.output();
    let mut seen = Seen::default();
    read_until(&mut output, &mut seen, |text| text.contains(PROMPT)).await;
    type_line(terminal.as_ref(), "print-password").await;
    read_until(&mut output, &mut seen, |text| {
        text.matches(PROMPT).count() == 2
    })
    .await;
    let closed = terminal.close(Duration::from_secs(2)).await;

    assert!(seen.text.contains("the password is [REDACTED]\r\n"));
    assert!(!seen.text.contains(PASSWORD));
    assert!(!seen.text.contains(&PASSWORD[..7]));
    assert_eq!(closed, TerminalEnding::Closed);
    core.runtime.shutdown().await;
}

#[tokio::test]
async fn a_typed_line_reaches_the_shell_whole_and_its_markers_come_back() {
    let hosted = hosted().await;
    let server = TestServer::start(Options::default()).await.unwrap();
    let key = register(&hosted, &server).await;
    let core = &hosted.core;
    let service = terminals(core);
    let conversation = conversation(core).await;
    let nonce = "0123456789abcdef0123456789abcdef";
    // A line as long as a typed setup, which the terminal plugin types after
    // the first prompt (plugins/terminal/docs/adr/0001-mark-commands-with-
    // shell-integration.md).
    let setup = format!(" eval '__mannie_n={nonce}; {}'", "x".repeat(6000));

    let mut terminal = service
        .open(spec(&conversation, &key))
        .await
        .expect("the terminal opens");
    let mut output = terminal.output();
    let mut seen = Seen::default();
    read_until(&mut output, &mut seen, |text| text.contains(PROMPT)).await;
    type_line(terminal.as_ref(), &setup).await;
    let marked = format!("\x1b]633;B;{nonce}\x07");
    read_until(&mut output, &mut seen, |text| text.contains(&marked)).await;
    type_line(terminal.as_ref(), "echo marked").await;
    let ended = format!("\x1b]633;D;0;{nonce}\x07");
    read_until(&mut output, &mut seen, |text| text.contains(&ended)).await;

    assert_eq!(server.shell_lines(), [setup.as_str(), "echo marked"]);
    assert!(
        seen.text
            .contains(&format!("\x1b]633;E;echo marked;{nonce}\x07"))
    );
    drop(terminal);
    core.runtime.shutdown().await;
}

#[tokio::test]
async fn a_host_whose_key_changed_opens_no_terminal_and_gets_no_password() {
    let hosted = hosted().await;
    let server = TestServer::start(Options::default()).await.unwrap();
    let key = register(&hosted, &server).await;
    let core = &hosted.core;
    let service = terminals(core);
    let conversation = conversation(core).await;
    server.replace_key().unwrap();

    let refused = service.open(spec(&conversation, &key)).await.unwrap_err();
    let mut lists = service.hosts();
    let listed = offered(&mut lists, &key, |host| {
        host.state != TerminalHostState::Ready
    })
    .await;

    assert_eq!(
        refused,
        TerminalOpenError::Failed {
            reason: "host_key_changed".to_owned()
        }
    );
    assert_eq!(server.auth_attempts(), 0);
    assert_eq!(
        listed.state,
        TerminalHostState::Unavailable {
            code: "key_changed".to_owned()
        }
    );
    core.runtime.shutdown().await;
}

#[tokio::test]
async fn an_approved_terminal_does_not_open_where_the_endpoint_changed() {
    let hosted = hosted().await;
    let server = TestServer::start(Options::default()).await.unwrap();
    let key = register(&hosted, &server).await;
    let core = &hosted.core;
    let service = terminals(core);
    let conversation = conversation(core).await;
    let approved = ExpectedHost {
        endpoint: format!("root@127.0.0.1:{}", server.port()),
        identity: server.fingerprint(),
    };

    let refused = service
        .open(spec(&conversation, &key).expecting(approved))
        .await
        .unwrap_err();

    assert_eq!(refused, TerminalOpenError::HostChanged);
    assert_eq!(server.auth_attempts(), 0);
    core.runtime.shutdown().await;
}

#[tokio::test]
async fn editing_the_host_ends_its_terminal_as_its_session_closed() {
    let hosted = hosted().await;
    let server = TestServer::start(Options::default()).await.unwrap();
    let key = register(&hosted, &server).await;
    let core = &hosted.core;
    let service = terminals(core);
    let conversation = conversation(core).await;
    let mut terminal = service
        .open(spec(&conversation, &key))
        .await
        .expect("the terminal opens");
    let mut output = terminal.output();
    let mut seen = Seen::default();
    read_until(&mut output, &mut seen, |text| text.contains(PROMPT)).await;

    let mut fields = host_fields("test-box", &server, "Edited.");
    fields["hostId"] = json!(key);
    fields["password"] = json!(null);
    hosted
        .invoke("update_host", fields)
        .await
        .expect("the host is edited");
    read_until(&mut output, &mut seen, |_text| false).await;

    assert_eq!(
        seen.ending,
        Some(TerminalEnding::Lost {
            code: "session_closed".to_owned()
        })
    );
    core.runtime.shutdown().await;
}

#[tokio::test]
async fn turning_ssh_off_ends_its_terminals_and_takes_its_hosts_off_the_list() {
    let hosted = hosted().await;
    let server = TestServer::start(Options::default()).await.unwrap();
    let key = register(&hosted, &server).await;
    let core = &hosted.core;
    let service = terminals(core);
    let conversation = conversation(core).await;
    let mut terminal = service
        .open(spec(&conversation, &key))
        .await
        .expect("the terminal opens");
    let mut output = terminal.output();
    let mut seen = Seen::default();
    read_until(&mut output, &mut seen, |text| text.contains(PROMPT)).await;

    hosted.turn_off().await;
    read_until(&mut output, &mut seen, |_text| false).await;
    let list = tokio::time::timeout(WAIT, service.hosts().next())
        .await
        .unwrap()
        .unwrap();
    let reopened = service.open(spec(&conversation, &key)).await.unwrap_err();

    assert_eq!(
        seen.ending,
        Some(TerminalEnding::Lost {
            code: "session_closed".to_owned()
        })
    );
    assert!(list.hosts.iter().all(|host| host.plugin != SSH));
    assert_eq!(reopened, TerminalOpenError::HostNotFound);
    core.runtime.shutdown().await;
}
