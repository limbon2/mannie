//! The Mannie worker ([ADR-0008], [ADR-0015]): the headless service that runs
//! on each host and owns it. In M0 it starts, logs, opens its store and waits
//! until it is stopped; the link and the agent runtime arrive with later
//! milestones.
//!
//! [ADR-0008]: ../../../docs/adr/0008-one-workspace-two-binaries.md
//! [ADR-0015]: ../../../docs/adr/0015-run-an-agent-runtime-on-every-machine.md

#[cfg(not(any(windows, target_os = "linux", target_os = "freebsd")))]
compile_error!("the worker runs on Windows, Linux and FreeBSD (ADR-0008)");

mod cli;
mod service;
mod worker;

use std::io::IsTerminal;
use std::path::Path;
use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;

use crate::cli::{Cli, Command, ServiceCommand};
use crate::worker::{Worker, WorkerOptions};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let outcome = match &cli.command {
        Command::Run(arguments) => run_in_foreground(&arguments.data_dir),
        Command::Service(ServiceCommand::Install(arguments)) => service::install(arguments),
        Command::Service(ServiceCommand::Uninstall) => service::uninstall(),
        #[cfg(windows)]
        Command::Service(ServiceCommand::Run(arguments)) => {
            service::run_as_windows_service(&arguments.data_dir)
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
fn run_in_foreground(data_directory: &Path) -> anyhow::Result<()> {
    let data_directory = std::path::absolute(data_directory).with_context(|| {
        format!(
            "the data folder {} is not a usable path",
            data_directory.display()
        )
    })?;
    let options = WorkerOptions {
        data_directory,
        console_logging: std::io::stderr().is_terminal(),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("could not start the async runtime")?;
    runtime.block_on(async {
        let worker = Worker::start(&options)?;
        let stop_signal = stop_requested().await;
        worker.stop();
        stop_signal.context("could not wait for a stop signal")
    })
}

/// Resolves when the terminal or the service manager asks the worker to stop.
#[cfg(unix)]
async fn stop_requested() -> std::io::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    tokio::select! {
        _ = terminate.recv() => {}
        _ = interrupt.recv() => {}
    }
    Ok(())
}

/// Resolves when the terminal asks the worker to stop: Ctrl+C, Ctrl+Break or
/// closing the console window.
#[cfg(windows)]
async fn stop_requested() -> std::io::Result<()> {
    use tokio::signal::windows::{ctrl_break, ctrl_c, ctrl_close};

    let mut interrupt = ctrl_c()?;
    let mut break_key = ctrl_break()?;
    let mut close = ctrl_close()?;
    tokio::select! {
        _ = interrupt.recv() => {}
        _ = break_key.recv() => {}
        _ = close.recv() => {}
    }
    Ok(())
}
