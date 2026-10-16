//! The Mannie worker ([ADR-0008], [ADR-0015], [ADR-0028]): the headless
//! service that runs on each host and owns it. It runs Mannie's agent
//! runtime under the host's ceiling for the master it is paired with, which
//! reaches it over the link ([ADR-0016]).
//!
//! [ADR-0008]: ../../../docs/adr/0008-one-workspace-two-binaries.md
//! [ADR-0015]: ../../../docs/adr/0015-run-an-agent-runtime-on-every-machine.md
//! [ADR-0016]: ../../../docs/adr/0016-link-machines-over-pinned-mtls-grpc.md
//! [ADR-0028]: ../../../docs/adr/0028-run-assigned-tasks-under-the-host-ceiling.md

#[cfg(not(any(windows, target_os = "linux", target_os = "freebsd")))]
compile_error!("the worker runs on Windows, Linux and FreeBSD (ADR-0008)");

mod access;
mod cli;
mod config;
mod hardening;
mod health;
mod pair;
mod paths;
mod run;
mod self_check;
mod service;
mod status;
mod update;
mod worker;

use std::future::Future;
use std::io::IsTerminal;
use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;

use crate::cli::{Cli, Command, RunArguments, ServiceCommand};
use crate::paths::Folders;
use crate::service::SystemService;
use crate::worker::{Worker, WorkerOptions};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let outcome = match &cli.command {
        Command::Run(arguments) => run_in_foreground(arguments),
        Command::Pair(arguments) => pair::pair(arguments, &SystemService),
        Command::Status(arguments) => status::status(arguments, &SystemService),
        Command::Facts => {
            print_facts();
            Ok(())
        }
        Command::SelfCheck(_arguments) => self_check::print(),
        Command::Update(arguments) => update::update(arguments),
        Command::Service(ServiceCommand::Install(arguments)) => service::install(arguments),
        Command::Service(ServiceCommand::Uninstall) => service::uninstall(),
        #[cfg(windows)]
        Command::Service(ServiceCommand::Run(arguments)) => {
            service::run_as_windows_service(arguments)
        }
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            #[expect(
                clippy::print_stderr,
                reason = "the command line reports its errors on standard error"
            )]
            {
                eprintln!("mannie-worker: {error:#}");
            }
            ExitCode::FAILURE
        }
    }
}

/// Runs the worker in this terminal until Ctrl+C or a termination signal.
fn run_in_foreground(arguments: &RunArguments) -> anyhow::Result<()> {
    let folders = Folders::of_this_system()?;
    let config = run::configuration(arguments, &folders)?;
    let options = WorkerOptions {
        config,
        folders,
        console_logging: std::io::stderr().is_terminal(),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("could not start the async runtime")?;
    runtime.block_on(async {
        // The signals are set up before the worker starts, so a stop that
        // comes while it starts is not lost.
        let stop = stop_requested().context("could not wait for a stop signal")?;
        let worker = Worker::start(&options).await?;
        worker.run_until(stop).await
    })
}

/// Prints the facts about this host that the worker reports to the master.
fn print_facts() {
    let facts = mannie_tools::SystemFacts::detect().host_facts();
    let listed = |names: &[String]| {
        if names.is_empty() {
            "none".to_owned()
        } else {
            names.join(", ")
        }
    };
    let lines = [
        format!("System: {}", facts.system),
        format!("Host name: {}", facts.hostname),
        format!("Shells: {}", listed(&facts.shells)),
        format!("Tools: {}", listed(&facts.tools)),
    ];
    #[expect(clippy::print_stdout, reason = "facts prints the facts on the console")]
    {
        println!("{}", lines.join("\n"));
    }
}

/// Resolves when the terminal or the service manager asks the worker to stop.
#[cfg(unix)]
fn stop_requested() -> std::io::Result<impl Future<Output = ()>> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    Ok(async move {
        tokio::select! {
            _ = terminate.recv() => {}
            _ = interrupt.recv() => {}
        }
    })
}

/// Resolves when the terminal asks the worker to stop: Ctrl+C, Ctrl+Break or
/// closing the console window.
#[cfg(windows)]
fn stop_requested() -> std::io::Result<impl Future<Output = ()>> {
    use tokio::signal::windows::{ctrl_break, ctrl_c, ctrl_close};

    let mut interrupt = ctrl_c()?;
    let mut break_key = ctrl_break()?;
    let mut close = ctrl_close()?;
    Ok(async move {
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = break_key.recv() => {}
            _ = close.recv() => {}
        }
    })
}
