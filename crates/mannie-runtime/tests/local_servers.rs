//! Local servers through the runtime's provider of the service
//! `local-servers` (ADR-0043, decision 5): a server reads what its plugin
//! sends and answers on its output, its secret reaches its own environment,
//! a stop closes its input and returns its end, and the end of its plugin,
//! a dropped server and the end of Mannie end it. A program, an argument, a
//! variable or a folder that breaks the rules starts nothing.

#![cfg(test)]
#![cfg(any(windows, target_os = "linux", target_os = "freebsd"))]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use mannie_plugin_sdk::context::HostProfile;
use mannie_plugin_sdk::host::{PluginBinding, ServiceProvider};
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::service::Service;
use mannie_plugin_sdk::services::local_servers::{
    LocalServer, LocalServerError, LocalServerSpec, LocalServers, LocalServersApi, ServerEnding,
    ServerOutput,
};
use mannie_runtime::testing::Harness;
use mannie_types::secret::Secret;
use serde_json::json;

/// The fake plugin.
const PLUGIN: &str = "servers";

/// How long a test waits for a server.
const WAIT: Duration = Duration::from_secs(30);

fn manifest() -> Manifest {
    serde_json::from_value(json!({
        "id": PLUGIN,
        "sdk": 1,
        "order": 10,
        "rust": { "entry": "mannie-servers" },
        "services": { "local-servers": 1 },
    }))
    .unwrap()
}

/// The runtime's provider of the service, and the service bound to the
/// fake plugin.
fn bound(harness: &Harness) -> (Arc<dyn ServiceProvider>, Arc<dyn LocalServersApi>) {
    let provider = harness
        .runtime
        .service_providers()
        .into_iter()
        .find(|provider| provider.name() == LocalServers::NAME)
        .expect("the runtime provides local servers");
    let id = PluginId::parse(PLUGIN).unwrap();
    let manifest = manifest();
    let service = provider
        .bind(&PluginBinding::new(
            &id,
            &manifest,
            HostProfile::Development,
        ))
        .unwrap()
        .get::<LocalServers>()
        .unwrap();
    (provider, service)
}

/// The system's shell as a server that runs `script`, by its absolute path.
fn shell(script: &str) -> LocalServerSpec {
    #[cfg(windows)]
    {
        let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap();
        let program = system.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        LocalServerSpec::new(
            program.display().to_string(),
            vec![
                "-NoLogo".to_owned(),
                "-NoProfile".to_owned(),
                "-NonInteractive".to_owned(),
                "-Command".to_owned(),
                script.to_owned(),
            ],
        )
    }
    #[cfg(not(windows))]
    {
        LocalServerSpec::new("/bin/sh", vec!["-c".to_owned(), script.to_owned()])
    }
}

/// A script that prints its secret's variable, then echoes each line of its
/// input until its input ends, and then exits with 7.
#[cfg(windows)]
const ECHO: &str = "[Console]::Out.WriteLine('token:'+$env:SERVER_TOKEN);[Console]::Out.Flush(); $in=[Console]::In; while(($line=$in.ReadLine()) -ne $null){[Console]::Out.WriteLine('echo:'+$line);[Console]::Out.Flush()}; exit 7";
#[cfg(not(windows))]
const ECHO: &str = "printf 'token:%s\\n' \"$SERVER_TOKEN\"; while IFS= read -r line; do printf 'echo:%s\\n' \"$line\"; done; exit 7";

/// Reads a server's output until it holds `wanted` or the server ended, and
/// returns what it read and the end, if one came.
async fn read_until(
    output: &mut BoxStream<'static, ServerOutput>,
    wanted: &str,
) -> (String, Option<ServerEnding>) {
    let mut text = String::new();
    loop {
        let item = tokio::time::timeout(WAIT, output.next())
            .await
            .expect("the server answers in time");
        match item {
            Some(ServerOutput::Bytes { bytes }) => {
                text.push_str(&String::from_utf8_lossy(&bytes));
                if text.contains(wanted) {
                    return (text, None);
                }
            }
            Some(ServerOutput::Ended { ending }) => return (text, Some(ending)),
            Some(_) | None => return (text, None),
        }
    }
}

/// Waits until a server's output ends, and returns its end.
async fn ending_of(output: &mut BoxStream<'static, ServerOutput>) -> Option<ServerEnding> {
    loop {
        let item = tokio::time::timeout(WAIT, output.next())
            .await
            .expect("the server ends in time");
        match item {
            Some(ServerOutput::Ended { ending }) => return Some(ending),
            Some(_) => {}
            None => return None,
        }
    }
}

async fn started(service: &Arc<dyn LocalServersApi>) -> Box<dyn LocalServer> {
    service
        .start(shell(ECHO).with_secret("SERVER_TOKEN", Secret::new("ghp_planted_9c2e".to_owned())))
        .await
        .unwrap()
}

#[tokio::test]
async fn a_server_gets_its_secret_answers_what_it_is_sent_and_stops_with_its_exit_code() {
    let harness = Harness::start().await;
    let (_provider, service) = bound(&harness);
    let mut server = started(&service).await;
    let mut output = server.output();

    let (greeting, _) = read_until(&mut output, "token:").await;
    server.send(b"first\nsecond\n".to_vec()).await.unwrap();
    let (echoed, _) = read_until(&mut output, "echo:second").await;
    let ending = server.stop(Duration::from_secs(10)).await;

    assert!(greeting.contains("token:ghp_planted_9c2e"), "{greeting}");
    assert!(echoed.contains("echo:first"), "{echoed}");
    assert_eq!(ending, ServerEnding::Exited { code: 7 });
    assert_eq!(ending_of(&mut output).await, Some(ending));
}

/// A server's secret goes into its own environment only: a server that
/// starts while it runs, without the secret, reads no such variable, and
/// neither does Mannie's own process (ADR-0043, decision 7).
#[tokio::test]
async fn a_servers_secret_reaches_only_its_own_server() {
    let harness = Harness::start().await;
    let (_provider, service) = bound(&harness);
    let mut with_secret = started(&service).await;
    let mut first = with_secret.output();
    let (greeting, _) = read_until(&mut first, "token:").await;
    let mut without = service.start(shell(ECHO)).await.unwrap();
    let mut second = without.output();

    let (other, _) = read_until(&mut second, "token:").await;
    without.send(b"line\n".to_vec()).await.unwrap();
    let (echoed, _) = read_until(&mut second, "echo:line").await;

    assert!(greeting.contains("token:ghp_planted_9c2e"), "{greeting}");
    assert!(!other.contains("ghp_planted_9c2e"), "{other}");
    assert!(echoed.contains("echo:line"), "{echoed}");
    assert!(std::env::var_os("SERVER_TOKEN").is_none());
    without.stop(Duration::from_secs(10)).await;
    with_secret.stop(Duration::from_secs(10)).await;
}

#[tokio::test]
async fn the_end_of_its_plugin_ends_a_server_and_its_input_takes_nothing_more() {
    let harness = Harness::start().await;
    let (provider, service) = bound(&harness);
    let mut server = started(&service).await;
    let mut output = server.output();
    read_until(&mut output, "token:").await;

    provider.release(&PluginId::parse(PLUGIN).unwrap()).await;

    assert!(ending_of(&mut output).await.is_some());
    assert_eq!(
        server.send(b"late\n".to_vec()).await,
        Err(LocalServerError::Ended)
    );
}

#[tokio::test]
async fn a_dropped_server_ends() {
    let harness = Harness::start().await;
    let (_provider, service) = bound(&harness);
    let mut server = started(&service).await;
    let mut output = server.output();
    read_until(&mut output, "token:").await;

    drop(server);

    assert!(ending_of(&mut output).await.is_some());
}

#[tokio::test]
async fn quitting_mannie_ends_every_server() {
    let harness = Harness::start().await;
    let (_provider, service) = bound(&harness);
    let mut server = started(&service).await;
    let mut output = server.output();
    read_until(&mut output, "token:").await;

    harness.runtime.shutdown().await;

    assert!(ending_of(&mut output).await.is_some());
    drop(server);
}

#[tokio::test]
async fn a_program_argument_variable_or_folder_past_the_rules_starts_nothing() {
    let harness = Harness::start().await;
    let (_provider, service) = bound(&harness);
    let refusal = |spec: LocalServerSpec| {
        let service = Arc::clone(&service);
        async move { service.start(spec).await.map(|_server| ()) }
    };

    assert_eq!(
        refusal(LocalServerSpec::new(
            "no-such-server-program-7d1a",
            Vec::new()
        ))
        .await,
        Err(LocalServerError::ProgramNotFound {
            program: "no-such-server-program-7d1a".to_owned()
        })
    );
    assert_eq!(
        refusal(LocalServerSpec::new("bin/server", Vec::new())).await,
        Err(LocalServerError::ProgramRefused {
            program: "bin/server".to_owned()
        })
    );
    assert_eq!(
        refusal(shell(ECHO).with_variable("LD_PRELOAD", "/tmp/a.so")).await,
        Err(LocalServerError::VariableRefused {
            name: "LD_PRELOAD".to_owned()
        })
    );
    assert_eq!(
        refusal(shell(ECHO).in_folder("relative/folder")).await,
        Err(LocalServerError::WorkingDirectoryInvalid)
    );
    assert_eq!(
        refusal(LocalServerSpec::new(
            shell(ECHO).program,
            vec!["a\0b".to_owned()]
        ))
        .await,
        Err(LocalServerError::ArgumentRefused)
    );
}
