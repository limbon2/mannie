//! Terminals through the runtime's provider of the service `terminals` and
//! its consumer of the point `terminal-hosts` (ADR-0053, decisions 3 and 4):
//! a shell of this machine runs what its plugin types and ends when closed,
//! a terminal needs a conversation that exists and keeps the limits, the
//! deletion of its conversation and the end of its plugin end it with their
//! reasons, the warning before quitting counts terminals that run a command,
//! and a host that another plugin offers is listed and opens, while a host
//! whose identity changed does not.

#![cfg(test)]
#![cfg(any(windows, target_os = "linux"))]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::stream::BoxStream;
use futures_util::{FutureExt, StreamExt};
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::HostProfile;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::host::{PluginBinding, ServiceProvider};
use mannie_plugin_sdk::ids::{Contributor, PluginId};
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::points::terminal_hosts::{
    OpenTerminal, PluginTerminalHostSource, TerminalConnection, TerminalHostEntry,
    TerminalHostSource, TerminalHosts, TerminalOpenFailure,
};
use mannie_plugin_sdk::service::Service;
use mannie_plugin_sdk::services::terminals::{
    ExpectedHost, MAX_TERMINALS_PER_CONVERSATION, ShellKind, Terminal, TerminalEnding,
    TerminalHostList, TerminalHostRef, TerminalIoError, TerminalOpenError, TerminalOutput,
    TerminalSize, TerminalSpec, Terminals, TerminalsApi,
};
use mannie_runtime::testing::{Harness, reply};
use mannie_types::chat::StartConversation;
use mannie_types::ids::ConversationId;
use serde_json::json;
use tokio::sync::mpsc;

/// The plugin that opens terminals.
const PLUGIN: &str = "terminal";

/// The plugin that offers a host.
const HOSTS: &str = "remote";

/// How long a test waits for a terminal.
const WAIT: Duration = Duration::from_secs(30);

/// How many pieces of output an echoing host's terminal holds.
const ECHO_QUEUE: usize = 64;

fn manifest() -> Manifest {
    serde_json::from_value(json!({
        "id": PLUGIN,
        "sdk": 1,
        "order": 10,
        "rust": { "entry": "mannie-terminal" },
        "services": { "terminals": 1 },
    }))
    .unwrap()
}

/// The runtime's provider of the service, and the service bound to the
/// plugin that opens terminals.
fn bound(harness: &Harness) -> (Arc<dyn ServiceProvider>, Arc<dyn TerminalsApi>) {
    let provider = harness
        .runtime
        .service_providers()
        .into_iter()
        .find(|provider| provider.name() == Terminals::NAME)
        .expect("the runtime provides terminals");
    let id = PluginId::parse(PLUGIN).unwrap();
    let manifest = manifest();
    let service = provider
        .bind(&PluginBinding::new(
            &id,
            &manifest,
            HostProfile::Development,
        ))
        .unwrap()
        .get::<Terminals>()
        .unwrap();
    (provider, service)
}

/// A conversation whose first reply ended.
async fn conversation(harness: &Harness) -> ConversationId {
    harness.model.push_reply(reply("Done."));
    let started = harness
        .runtime
        .start_conversation(StartConversation::new("OWNER: hello"))
        .await
        .unwrap();
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    started.conversation_id
}

/// The first list of hosts.
async fn hosts(service: &Arc<dyn TerminalsApi>) -> TerminalHostList {
    tokio::time::timeout(WAIT, service.hosts().next())
        .await
        .expect("the list of hosts comes at once")
        .expect("the list of hosts has an item")
}

/// A plain shell of this machine: `cmd.exe` on Windows, `sh` elsewhere.
async fn plain_shell(service: &Arc<dyn TerminalsApi>) -> TerminalHostRef {
    let list = hosts(service).await;
    let wanted = if cfg!(windows) {
        ShellKind::Cmd
    } else {
        ShellKind::Sh
    };
    let shell = list
        .shells
        .iter()
        .find(|shell| shell.kind == wanted)
        .or_else(|| list.shells.iter().find(|shell| shell.default))
        .expect("this machine has a shell");
    TerminalHostRef::ThisMachine {
        shell: shell.program.clone(),
    }
}

fn size() -> TerminalSize {
    TerminalSize::new(100, 30).unwrap()
}

/// A line that a plain shell answers with 42, which the typed line itself
/// does not show.
fn product() -> &'static [u8] {
    if cfg!(windows) {
        b"set /a 6*7\r"
    } else {
        b"echo $((6*7))\n"
    }
}

/// Reads a terminal's output until it holds `wanted` or the terminal ended,
/// and returns what it read and the end, if one came.
async fn read_until(
    output: &mut BoxStream<'static, TerminalOutput>,
    wanted: &str,
) -> (String, Option<TerminalEnding>) {
    let mut text = String::new();
    loop {
        let item = tokio::time::timeout(WAIT, output.next())
            .await
            .expect("the terminal answers in time");
        match item {
            Some(TerminalOutput::Bytes { bytes }) => {
                text.push_str(&String::from_utf8_lossy(&bytes));
                if !wanted.is_empty() && text.contains(wanted) {
                    return (text, None);
                }
            }
            Some(TerminalOutput::Ended { ending }) => return (text, Some(ending)),
            Some(_) | None => return (text, None),
        }
    }
}

#[tokio::test]
async fn a_shell_of_this_machine_runs_what_is_typed_and_ends_when_closed() {
    let harness = Harness::start().await;
    let (_provider, service) = bound(&harness);
    let conversation = conversation(&harness).await;
    let shell = plain_shell(&service).await;

    let mut terminal = service
        .open(TerminalSpec::new(conversation.as_str(), shell, size()))
        .await
        .unwrap();
    let mut output = terminal.output();
    terminal.write(product().to_vec()).await.unwrap();

    let (text, ended) = read_until(&mut output, "42").await;
    assert!(text.contains("42"), "{text:?}");
    assert_eq!(ended, None);

    let ending = terminal.close(Duration::from_secs(2)).await;
    assert!(
        matches!(
            ending,
            TerminalEnding::Closed | TerminalEnding::Exited { .. }
        ),
        "{ending:?}"
    );
    let (_rest, ended) = read_until(&mut output, "").await;
    assert!(
        ended.is_some(),
        "the output ends with how the terminal ended"
    );
}

#[tokio::test]
async fn a_terminal_needs_its_conversation_and_keeps_the_limit_of_a_conversation() {
    let harness = Harness::start().await;
    let (_provider, service) = bound(&harness);
    let shell = plain_shell(&service).await;

    let missing = service
        .open(TerminalSpec::new(
            "0199e0f2-7a1b-7c3d-8e4f-0000000009ff",
            shell.clone(),
            size(),
        ))
        .await
        .err();
    assert_eq!(missing, Some(TerminalOpenError::ConversationNotFound));

    let conversation = conversation(&harness).await;
    let mut open: Vec<Box<dyn Terminal>> = Vec::new();
    for _ in 0..MAX_TERMINALS_PER_CONVERSATION {
        open.push(
            service
                .open(TerminalSpec::new(
                    conversation.as_str(),
                    shell.clone(),
                    size(),
                ))
                .await
                .unwrap(),
        );
    }
    let one_more = service
        .open(TerminalSpec::new(
            conversation.as_str(),
            shell.clone(),
            size(),
        ))
        .await
        .err();
    assert_eq!(
        one_more,
        Some(TerminalOpenError::TooMany {
            limit: MAX_TERMINALS_PER_CONVERSATION
        })
    );
    for terminal in open {
        terminal.close(Duration::ZERO).await;
    }
    let unlisted = service
        .open(TerminalSpec::new(
            conversation.as_str(),
            TerminalHostRef::ThisMachine {
                shell: PathBuf::from(if cfg!(windows) {
                    r"C:\not-a-shell.exe"
                } else {
                    "/not/a/shell"
                }),
            },
            size(),
        ))
        .await
        .err();
    assert_eq!(unlisted, Some(TerminalOpenError::ShellNotListed));
}

#[tokio::test]
async fn deleting_its_conversation_or_its_plugin_going_off_ends_a_terminal() {
    let harness = Harness::start().await;
    let (provider, service) = bound(&harness);
    let shell = plain_shell(&service).await;

    let deleted = conversation(&harness).await;
    let mut terminal = service
        .open(TerminalSpec::new(deleted.as_str(), shell.clone(), size()))
        .await
        .unwrap();
    let mut output = terminal.output();
    harness.runtime.delete_conversation(&deleted).await.unwrap();
    let (_text, ended) = read_until(&mut output, "").await;
    assert_eq!(ended, Some(TerminalEnding::ConversationDeleted));
    assert_eq!(
        terminal.write(b"echo late\n".to_vec()).await,
        Err(TerminalIoError::Ended)
    );

    let kept = conversation(&harness).await;
    let mut terminal = service
        .open(TerminalSpec::new(kept.as_str(), shell, size()))
        .await
        .unwrap();
    let mut output = terminal.output();
    provider
        .release(&Contributor::plugin(PluginId::parse(PLUGIN).unwrap()))
        .await;
    let (_text, ended) = read_until(&mut output, "").await;
    assert_eq!(ended, Some(TerminalEnding::PluginOff));
}

#[tokio::test]
async fn the_warning_before_quitting_counts_terminals_that_run_a_command() {
    let harness = Harness::start().await;
    let (_provider, service) = bound(&harness);
    let conversation = conversation(&harness).await;
    let shell = plain_shell(&service).await;
    let terminal = service
        .open(TerminalSpec::new(conversation.as_str(), shell, size()))
        .await
        .unwrap();

    assert_eq!(harness.runtime.running_terminal_count(), 0);
    terminal.set_running(true);
    assert_eq!(harness.runtime.running_terminal_count(), 1);
    terminal.set_running(false);
    assert_eq!(harness.runtime.running_terminal_count(), 0);
    terminal.close(Duration::ZERO).await;
}

/// A source of one host, whose terminals echo what is typed.
#[derive(Debug, Default)]
struct EchoHosts {
    opened: Mutex<Vec<OpenTerminal>>,
}

impl TerminalHostSource for EchoHosts {
    fn hosts(&self) -> BoxStream<'static, Vec<TerminalHostEntry>> {
        let host = TerminalHostEntry::new(
            "host-1".to_owned(),
            "build-box".to_owned(),
            "deploy@build-box.lan:22".to_owned(),
            "SHA256:pinned".to_owned(),
        )
        .with_shell(ShellKind::Bash);
        // The list stays until the plugin stops.
        futures_util::stream::once(async move { vec![host] })
            .chain(futures_util::stream::pending())
            .boxed()
    }

    fn open(
        &self,
        request: OpenTerminal,
    ) -> BoxFuture<'static, Result<Box<dyn TerminalConnection>, TerminalOpenFailure>> {
        self.opened.lock().unwrap().push(request);
        let (echo, output) = mpsc::channel(ECHO_QUEUE);
        let connection: Box<dyn TerminalConnection> = Box::new(EchoConnection {
            echo,
            output: Some(output),
        });
        async move { Ok(connection) }.boxed()
    }
}

#[derive(Debug)]
struct EchoConnection {
    echo: mpsc::Sender<TerminalOutput>,
    output: Option<mpsc::Receiver<TerminalOutput>>,
}

impl TerminalConnection for EchoConnection {
    fn output(&mut self) -> BoxStream<'static, TerminalOutput> {
        let Some(output) = self.output.take() else {
            return futures_util::stream::empty().boxed();
        };
        futures_util::stream::unfold(output, |mut output| async move {
            output.recv().await.map(|item| (item, output))
        })
        .boxed()
    }

    fn write(&self, bytes: Vec<u8>) -> BoxFuture<'static, Result<(), TerminalIoError>> {
        let sent = self.echo.try_send(TerminalOutput::Bytes { bytes });
        async move { sent.map_err(|_closed| TerminalIoError::Ended) }.boxed()
    }

    fn resize(&self, _size: TerminalSize) -> BoxFuture<'static, Result<(), TerminalIoError>> {
        async { Ok(()) }.boxed()
    }

    fn close(self: Box<Self>, _grace: Duration) -> BoxFuture<'static, TerminalEnding> {
        // A reader that is gone needs no end.
        let _sent = self.echo.try_send(TerminalOutput::Ended {
            ending: TerminalEnding::Closed,
        });
        async { TerminalEnding::Closed }.boxed()
    }
}

#[tokio::test]
async fn a_host_that_a_plugin_offers_is_listed_and_opens_unless_it_changed() {
    let harness = Harness::start().await;
    let (_provider, service) = bound(&harness);
    let conversation = conversation(&harness).await;
    let source = Arc::new(EchoHosts::default());
    let shared: Arc<dyn TerminalHostSource> = Arc::<EchoHosts>::clone(&source);
    let hosts_plugin = Contributor::plugin(PluginId::parse(HOSTS).unwrap());
    let consumer = harness
        .runtime
        .contribution_consumers()
        .into_iter()
        .next()
        .unwrap();
    consumer
        .add(
            &hosts_plugin,
            Contributions::new().add::<TerminalHosts>(PluginTerminalHostSource::new(HOSTS, shared)),
        )
        .unwrap();

    let list = hosts(&service).await;
    let offered = list
        .hosts
        .iter()
        .find(|host| host.plugin == HOSTS)
        .expect("the plugin's host is listed");
    assert_eq!(offered.key, "host-1");
    assert_eq!(offered.label, "build-box");
    assert_eq!(offered.shell, Some(ShellKind::Bash));

    let host = TerminalHostRef::Plugin {
        plugin: HOSTS.to_owned(),
        key: "host-1".to_owned(),
    };
    let expect = |identity: &str| ExpectedHost {
        endpoint: "deploy@build-box.lan:22".to_owned(),
        identity: identity.to_owned(),
    };
    let mut terminal = service
        .open(
            TerminalSpec::new(conversation.as_str(), host.clone(), size())
                .expecting(expect("SHA256:pinned")),
        )
        .await
        .unwrap();
    let mut output = terminal.output();
    terminal.write(b"uptime\r".to_vec()).await.unwrap();
    let (text, _) = read_until(&mut output, "uptime").await;
    assert!(text.contains("uptime"));
    assert_eq!(
        source.opened.lock().unwrap()[0].expect,
        Some(expect("SHA256:pinned"))
    );

    let changed = service
        .open(
            TerminalSpec::new(conversation.as_str(), host, size())
                .expecting(expect("SHA256:other")),
        )
        .await
        .err();
    assert_eq!(changed, Some(TerminalOpenError::HostChanged));
    let unknown = service
        .open(TerminalSpec::new(
            conversation.as_str(),
            TerminalHostRef::Plugin {
                plugin: HOSTS.to_owned(),
                key: "host-2".to_owned(),
            },
            size(),
        ))
        .await
        .err();
    assert_eq!(unknown, Some(TerminalOpenError::HostNotFound));

    consumer.remove(&hosts_plugin);
    let after = hosts(&service).await;
    assert!(after.hosts.iter().all(|host| host.plugin != HOSTS));
    terminal.close(Duration::ZERO).await;
}
