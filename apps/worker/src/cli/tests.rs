use std::path::Path;

use clap::{CommandFactory, Parser};

use super::{Cli, Command, ServiceCommand};

#[test]
fn the_command_line_definition_is_consistent() {
    Cli::command().debug_assert();
}

#[test]
fn run_takes_the_data_folder() {
    let cli = Cli::try_parse_from(["mannie-worker", "run", "--data-dir", "data"]).unwrap();

    let Command::Run(arguments) = cli.command else {
        panic!("expected the run command")
    };
    assert_eq!(arguments.data_dir, Path::new("data"));
}

#[test]
fn run_needs_the_data_folder() {
    let error = Cli::try_parse_from(["mannie-worker", "run"]).unwrap_err();

    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );
}

#[cfg(unix)]
#[test]
fn install_takes_the_data_folder_and_the_account() {
    let cli = Cli::try_parse_from([
        "mannie-worker",
        "service",
        "install",
        "--data-dir",
        "/var/lib/mannie-worker",
        "--user",
        "mannie",
    ])
    .unwrap();

    let Command::Service(ServiceCommand::Install(arguments)) = cli.command else {
        panic!("expected service install")
    };
    assert_eq!(arguments.data_dir, Path::new("/var/lib/mannie-worker"));
    assert_eq!(arguments.user, "mannie");
}

#[cfg(unix)]
#[test]
fn install_needs_the_account() {
    let error = Cli::try_parse_from([
        "mannie-worker",
        "service",
        "install",
        "--data-dir",
        "/var/lib/mannie-worker",
    ])
    .unwrap_err();

    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );
}

#[cfg(windows)]
#[test]
fn install_takes_the_data_folder() {
    let cli = Cli::try_parse_from([
        "mannie-worker",
        "service",
        "install",
        "--data-dir",
        r"C:\ProgramData\Mannie\worker",
    ])
    .unwrap();

    let Command::Service(ServiceCommand::Install(arguments)) = cli.command else {
        panic!("expected service install")
    };
    assert_eq!(
        arguments.data_dir,
        Path::new(r"C:\ProgramData\Mannie\worker")
    );
}

#[test]
fn uninstall_takes_no_arguments() {
    let cli = Cli::try_parse_from(["mannie-worker", "service", "uninstall"]).unwrap();

    assert!(matches!(
        cli.command,
        Command::Service(ServiceCommand::Uninstall)
    ));
}
