use std::path::Path;

use clap::{CommandFactory, Parser};

use super::{Cli, Command, ServiceCommand};

fn parse(arguments: &[&str]) -> Command {
    Cli::try_parse_from(std::iter::once("mannie-worker").chain(arguments.iter().copied()))
        .unwrap()
        .command
}

#[test]
fn the_command_line_definition_is_consistent() {
    Cli::command().debug_assert();
}

#[test]
fn run_takes_a_configuration_a_data_folder_and_the_writable_override() {
    let Command::Run(arguments) = parse(&[
        "run",
        "--config",
        "worker.toml",
        "--data-dir",
        "data",
        "--allow-writable-config",
    ]) else {
        panic!("expected the run command")
    };

    assert_eq!(
        arguments.config.config.as_deref(),
        Some(Path::new("worker.toml"))
    );
    assert_eq!(arguments.data_dir.as_deref(), Some(Path::new("data")));
    assert!(arguments.allow_writable_config);
}

#[test]
fn run_uses_the_systems_configuration_by_default() {
    let Command::Run(arguments) = parse(&["run"]) else {
        panic!("expected the run command")
    };

    assert_eq!(arguments.config.config, None);
    assert_eq!(arguments.data_dir, None);
    assert!(!arguments.allow_writable_config);
}

#[test]
fn pair_takes_reset() {
    let Command::Pair(plain) = parse(&["pair"]) else {
        panic!("expected pair")
    };
    let Command::Pair(reset) = parse(&["pair", "--reset", "--config", "w.toml"]) else {
        panic!("expected pair")
    };

    assert!(!plain.reset);
    assert!(reset.reset);
    assert_eq!(reset.config.config.as_deref(), Some(Path::new("w.toml")));
}

#[test]
fn self_check_needs_json() {
    let error = Cli::try_parse_from(["mannie-worker", "self-check"]).unwrap_err();

    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );
    assert!(matches!(
        parse(&["self-check", "--json"]),
        Command::SelfCheck(_)
    ));
}

#[test]
fn update_takes_the_archive_and_check() {
    let Command::Update(arguments) = parse(&[
        "update",
        "mannie-worker-0.4.1-linux-x86_64.tar.gz",
        "--check",
    ]) else {
        panic!("expected update")
    };

    assert_eq!(
        arguments.archive,
        Path::new("mannie-worker-0.4.1-linux-x86_64.tar.gz")
    );
    assert!(arguments.check);
}

#[test]
fn status_and_facts_parse() {
    assert!(matches!(parse(&["status"]), Command::Status(_)));
    assert!(matches!(parse(&["facts"]), Command::Facts));
}

#[cfg(unix)]
#[test]
fn install_takes_the_account_and_a_configuration() {
    let Command::Service(ServiceCommand::Install(arguments)) = parse(&[
        "service",
        "install",
        "--user",
        "mannie-worker",
        "--config",
        "/etc/mannie-worker/worker.toml",
    ]) else {
        panic!("expected service install")
    };

    assert_eq!(arguments.user, "mannie-worker");
    assert_eq!(
        arguments.config.config.as_deref(),
        Some(Path::new("/etc/mannie-worker/worker.toml"))
    );
}

#[cfg(unix)]
#[test]
fn install_needs_the_account() {
    let error = Cli::try_parse_from(["mannie-worker", "service", "install"]).unwrap_err();

    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );
}

#[cfg(windows)]
#[test]
fn install_takes_a_configuration_and_the_service_runs_with_it() {
    let Command::Service(ServiceCommand::Install(install)) = parse(&["service", "install"]) else {
        panic!("expected service install")
    };
    let Command::Service(ServiceCommand::Run(run)) = parse(&[
        "service",
        "run",
        "--config",
        r"C:\ProgramData\Mannie\worker\worker.toml",
    ]) else {
        panic!("expected service run")
    };

    assert_eq!(install.config.config, None);
    assert_eq!(
        run.config.config.as_deref(),
        Some(Path::new(r"C:\ProgramData\Mannie\worker\worker.toml"))
    );
}
