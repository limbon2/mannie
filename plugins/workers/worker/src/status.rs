//! `status` (plugins/workers/docs/service.md): whether the configuration
//! is valid, whether the service runs, whether the worker is paired and
//! with which key, and where it listens. It never opens the database.

use anyhow::bail;

use crate::cli::ConfigArgument;
use crate::health;
use crate::pair::{paired_pin, reachable_address};
use crate::paths::Folders;
use crate::run;
use crate::service::ServiceControl;

/// Prints the worker's status, and fails when the configuration is not
/// valid.
pub(crate) fn status(
    argument: &ConfigArgument,
    service: &dyn ServiceControl,
) -> anyhow::Result<()> {
    let folders = Folders::of_this_system()?;
    let file = run::config_file(argument, &folders);
    let config = run::installed_configuration(argument, &folders);
    match &config {
        Ok(_config) => say(&format!("Configuration: {} is valid.", file.display())),
        Err(error) => say(&format!("Configuration: {error:#}")),
    }
    let service_line = match (service.is_installed(), service.is_running()) {
        (Ok(false), _) => "not installed".to_owned(),
        (Ok(true), Ok(true)) => "running".to_owned(),
        (Ok(true), Ok(false)) => "installed, not running".to_owned(),
        (Err(error), _) | (Ok(true), Err(error)) => format!("unknown: {error:#}"),
    };
    say(&format!("Service: {service_line}."));
    let Ok(config) = config else {
        bail!("the configuration is not valid");
    };
    say(&format!(
        "Listens on: {} (reach it as {}).",
        config.listen,
        reachable_address(config.listen)
    ));
    say(&format!("Data folder: {}.", config.data_dir.display()));
    let pairing = match paired_pin(&config.data_dir) {
        Ok(Some(pin)) => format!(
            "paired with the Mannie whose key starts with {}",
            pin.short()
        ),
        Ok(None) => "not paired; run `mannie-worker pair`".to_owned(),
        Err(error) => format!(
            "unknown, the data folder cannot be read ({error}); run as root or administrator"
        ),
    };
    say(&format!("Pairing: {pairing}."));
    match health::read(&config.data_dir) {
        Some(running) => say(&format!(
            "Program: version {} runs as process {}.",
            running.version, running.process_id
        )),
        None => say("Program: no worker runs with this data folder."),
    }
    Ok(())
}

#[expect(
    clippy::print_stdout,
    reason = "status prints its lines on the console"
)]
fn say(text: &str) {
    println!("{text}");
}
