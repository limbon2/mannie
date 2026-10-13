//! The worker as a systemd unit.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use anyhow::{Context, bail};

use super::{SERVICE_NAME, report, unix};

/// Where `install` writes the unit.
const UNIT_PATH: &str = "/etc/systemd/system/mannie-worker.service";

pub(super) fn install(executable: &Path, data_directory: &Path, user: &str) -> anyhow::Result<()> {
    let executable = unix::plain_path(executable, "worker's executable")?;
    let data_directory = unix::plain_path(data_directory, "data folder")?;
    unix::existing_account(user)?;
    let unit = unit_file(executable, data_directory, user);
    write_unit(&unit)?;
    report(&format!("Wrote the systemd unit {UNIT_PATH}."));
    unix::run("systemctl", &["daemon-reload".as_ref()])?;
    unix::run("systemctl", &["enable".as_ref(), "--now".as_ref(), unit_name().as_ref()]).with_context(|| {
        format!("the unit is installed but did not start. `systemctl status {SERVICE_NAME}` and `journalctl -u {SERVICE_NAME}` show why")
    })?;
    report(&format!(
        "Started the service {SERVICE_NAME} as {user}. It starts with the system."
    ));
    Ok(())
}

pub(super) fn uninstall() -> anyhow::Result<()> {
    if !Path::new(UNIT_PATH).exists() {
        bail!("the service {SERVICE_NAME} is not installed: {UNIT_PATH} does not exist");
    }
    unix::run(
        "systemctl",
        &["disable".as_ref(), "--now".as_ref(), unit_name().as_ref()],
    )?;
    report(&format!("Stopped the service {SERVICE_NAME}."));
    fs::remove_file(UNIT_PATH).with_context(|| format!("could not remove {UNIT_PATH}"))?;
    unix::run("systemctl", &["daemon-reload".as_ref()])?;
    report(&format!(
        "Removed the systemd unit {UNIT_PATH}. The data folder was kept."
    ));
    Ok(())
}

fn unit_name() -> String {
    format!("{SERVICE_NAME}.service")
}

/// Writes the unit, refusing to replace one that exists.
fn write_unit(unit: &str) -> anyhow::Result<()> {
    let opened = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(UNIT_PATH);
    let mut file = match opened {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            bail!(
                "the service {SERVICE_NAME} is already installed ({UNIT_PATH}). Run `mannie-worker service uninstall` first"
            )
        }
        Err(error) if error.kind() == ErrorKind::PermissionDenied => {
            return Err(error).context(format!(
                "could not write {UNIT_PATH}. Run the command as root"
            ));
        }
        Err(error) => return Err(error).context(format!("could not write {UNIT_PATH}")),
    };
    file.write_all(unit.as_bytes())
        .and_then(|()| file.sync_all())
        .with_context(|| format!("could not write {UNIT_PATH}"))
}

/// The unit. The values are plain (see `unix::plain_path`), so none needs quoting.
fn unit_file(executable: &str, data_directory: &str, user: &str) -> String {
    format!(
        "\
# Written by `mannie-worker service install`; `mannie-worker service uninstall` removes it.
[Unit]
Description=Mannie worker
After=network-online.target
Wants=network-online.target
# A worker that keeps failing to start stops being restarted after five tries.
StartLimitIntervalSec=300
StartLimitBurst=5

[Service]
Type=exec
User={user}
# Files the worker creates are readable only by its account.
UMask=0077
# Runs as root (+) to create the data folder for the account, readable only by it.
ExecStartPre=+/usr/bin/install -d -o {user} -m 0700 {data_directory}
ExecStart={executable} run --data-dir {data_directory}
Restart=on-failure
RestartSec=10
TimeoutStopSec=30

[Install]
WantedBy=multi-user.target
"
    )
}

#[cfg(test)]
mod tests;
