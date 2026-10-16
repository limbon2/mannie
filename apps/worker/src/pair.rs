//! Pairing from the host (docs/features/workers/pairing-and-link.md,
//! pairing): `pair` writes a one-time code into the data folder, which the
//! running worker checks, and prints it with the worker's address; `pair
//! --reset` forgets the paired master. Both run as root or with
//! administrator rights, and neither opens the database.

use std::io;
use std::net::SocketAddr;
use std::path::Path;

use anyhow::{Context, bail};
use mannie_link::keys::KeyPin;
use mannie_link::pairing::CODE_LIFETIME;
use mannie_link::worker::{CODE_FILE, PIN_FILE};
use mannie_types::clock::{Clock, SystemClock};

use crate::cli::PairArguments;
use crate::config::Config;
use crate::paths::Folders;
use crate::run;
use crate::service::ServiceControl;

/// The pin of the paired master, if the worker is paired.
pub(crate) fn paired_pin(data: &Path) -> io::Result<Option<KeyPin>> {
    match std::fs::read_to_string(data.join(PIN_FILE)) {
        Ok(text) => Ok(KeyPin::parse(&text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// How the owner reaches the worker: its address, or this host's name with
/// the port when it listens on every address.
pub(crate) fn reachable_address(listen: SocketAddr) -> String {
    if listen.ip().is_unspecified() {
        let host = mannie_tools::SystemFacts::detect().host_facts().hostname;
        format!("{host}:{}", listen.port())
    } else {
        listen.to_string()
    }
}

/// Writes a pairing code, or forgets the paired master with `--reset`.
pub(crate) fn pair(arguments: &PairArguments, service: &dyn ServiceControl) -> anyhow::Result<()> {
    let folders = Folders::of_this_system()?;
    let config = run::installed_configuration(&arguments.config, &folders)?;
    let data = &config.data_dir;
    if !data.is_dir() {
        bail!(
            "the data folder {} does not exist yet. Start the worker first; it creates the folder",
            data.display()
        );
    }
    if arguments.reset {
        return reset(&config, service);
    }
    let pinned = paired_pin(data).with_context(|| {
        format!(
            "could not read the data folder {}. Run the command as root or administrator",
            data.display()
        )
    })?;
    if let Some(pin) = pinned {
        bail!(
            "the worker is paired with a Mannie already, whose key starts with {}. Remove the worker in that Mannie, or run `mannie-worker pair --reset` to pair it with another",
            pin.short()
        );
    }
    let code = mannie_link::worker::write_code(data, SystemClock.now()).with_context(|| {
        format!(
            "could not write the pairing code into {}. Run the command as root or administrator",
            data.display()
        )
    })?;
    // The service's account reads the code and deletes it once it is used.
    #[cfg(unix)]
    crate::access::give_to_owner_of(&data.join(CODE_FILE), data)
        .context("could not give the pairing code to the worker's account")?;
    say(&format!(
        "Pair this worker in Mannie's Workers settings with:\n\n  Address: {}\n  Code:    {}\n\nThe code works once, for {} minutes. If Mannie cannot reach the address, use this host's IP address with port {}.",
        reachable_address(config.listen),
        code.display(),
        CODE_LIFETIME.as_secs() / 60,
        config.listen.port()
    ));
    Ok(())
}

/// Forgets the paired master and the code, and restarts the service, which
/// drops every lease and interrupts the tasks.
fn reset(config: &Config, service: &dyn ServiceControl) -> anyhow::Result<()> {
    let installed = service.is_installed()?;
    if installed && service.is_running()? {
        service.stop().context("could not stop the service")?;
    }
    for file in [PIN_FILE, CODE_FILE] {
        let path = config.data_dir.join(file);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "could not delete {}. Run the command as root or administrator",
                        path.display()
                    )
                });
            }
        }
    }
    if installed {
        service
            .start()
            .context("forgot the paired Mannie, but the service did not start again")?;
        say(
            "Forgot the paired Mannie and restarted the service, which dropped its leases and interrupted its tasks. Run `mannie-worker pair` to pair it again.",
        );
    } else {
        say(
            "Forgot the paired Mannie. Restart the worker if it runs in a terminal, then run `mannie-worker pair` to pair it again.",
        );
    }
    Ok(())
}

#[expect(
    clippy::print_stdout,
    reason = "pair prints the code and its result on the console"
)]
fn say(text: &str) {
    println!("{text}");
}
