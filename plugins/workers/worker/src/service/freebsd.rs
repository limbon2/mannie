//! The worker as an rc.d service. daemon(8) runs it under the account,
//! restarts it when it exits, and sends its standard output to syslog.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use anyhow::{Context, bail};

use super::{report, unix};
use crate::config::Config;

/// The rc.d name, a shell variable name.
const RC_NAME: &str = "mannie_worker";

/// Where `install` writes the rc.d script.
const SCRIPT_PATH: &str = "/usr/local/etc/rc.d/mannie_worker";

/// The service command.
const SERVICE: &str = "/usr/sbin/service";

pub(super) fn install(
    program: &Path,
    config_file: &Path,
    config: &Config,
    user: &str,
) -> anyhow::Result<()> {
    let program = unix::plain_path(program, "worker's program")?;
    let config_file = unix::plain_path(config_file, "configuration")?;
    let data_directory = unix::plain_path(&config.data_dir, "data folder")?;
    unix::existing_account(user)?;
    write_script(&rc_script(program, config_file, data_directory, user))?;
    report(&format!("Wrote the rc.d script {SCRIPT_PATH}."));
    unix::run(
        "/usr/sbin/sysrc",
        &[format!("{RC_NAME}_enable=YES").as_ref()],
    )?;
    unix::run(SERVICE, &[RC_NAME.as_ref(), "start".as_ref()]).with_context(|| {
        format!("the service is installed but did not start. `service {RC_NAME} status` and /var/log/messages show why")
    })?;
    report(&format!(
        "Started the service {RC_NAME} as {user}. It starts with the system."
    ));
    Ok(())
}

pub(super) fn uninstall() -> anyhow::Result<()> {
    if !is_installed()? {
        bail!("the service {RC_NAME} is not installed: {SCRIPT_PATH} does not exist");
    }
    if is_running()? {
        stop()?;
        report(&format!("Stopped the service {RC_NAME}."));
    }
    unix::run(
        "/usr/sbin/sysrc",
        &["-x".as_ref(), format!("{RC_NAME}_enable").as_ref()],
    )?;
    fs::remove_file(SCRIPT_PATH).with_context(|| format!("could not remove {SCRIPT_PATH}"))?;
    report(&format!("Removed the rc.d script {SCRIPT_PATH}."));
    Ok(())
}

pub(super) fn is_installed() -> anyhow::Result<bool> {
    Path::new(SCRIPT_PATH)
        .try_exists()
        .with_context(|| format!("could not look for {SCRIPT_PATH}"))
}

// The `one` forms work whether or not rc.conf enables the service.

pub(super) fn is_running() -> anyhow::Result<bool> {
    unix::succeeds(SERVICE, &[RC_NAME.as_ref(), "onestatus".as_ref()])
}

pub(super) fn stop() -> anyhow::Result<()> {
    unix::run(SERVICE, &[RC_NAME.as_ref(), "onestop".as_ref()])
}

pub(super) fn start() -> anyhow::Result<()> {
    unix::run(SERVICE, &[RC_NAME.as_ref(), "onestart".as_ref()])
}

/// Writes the script, refusing to replace one that exists.
fn write_script(script: &str) -> anyhow::Result<()> {
    let opened = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o555)
        .open(SCRIPT_PATH);
    let mut file = match opened {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            bail!(
                "the service {RC_NAME} is already installed ({SCRIPT_PATH}). Run `mannie-worker service uninstall` first"
            )
        }
        Err(error) if error.kind() == ErrorKind::PermissionDenied => {
            return Err(error).context(format!(
                "could not write {SCRIPT_PATH}. Run the command as root"
            ));
        }
        Err(error) => return Err(error).context(format!("could not write {SCRIPT_PATH}")),
    };
    file.write_all(script.as_bytes())
        .and_then(|()| file.sync_all())
        .with_context(|| format!("could not write {SCRIPT_PATH}"))
}

/// The rc.d script. The values are plain (see `unix::plain_path`), so none
/// needs quoting. rc.conf can override the account, the configuration and
/// the data folder.
///
/// daemon(8) runs as root and starts the worker as the account with `-u`,
/// which needs root and also sets `HOME`, `USER` and `SHELL`, so a restart
/// works too. The account's variable must not be `mannie_worker_user`:
/// rc.subr would run daemon(8) itself as that account through su(1). `-f`
/// sends daemon's own standard streams to `/dev/null`; otherwise it keeps
/// the pipes that `service start` inherited, and `service install` waits on
/// them forever.
fn rc_script(program: &str, config_file: &str, data_directory: &str, user: &str) -> String {
    format!(
        r#"#!/bin/sh
# Written by `mannie-worker service install`; `mannie-worker service uninstall` removes it.
#
# PROVIDE: mannie_worker
# REQUIRE: LOGIN NETWORKING
# KEYWORD: shutdown
#
# mannie_worker_enable (bool):     Start the worker with the system. Default: NO.
# mannie_worker_account (string):  The account that runs the worker.
# mannie_worker_config (string):   The worker's configuration.
# mannie_worker_data_dir (string): The worker's data folder, as the configuration names it.

. /etc/rc.subr

name="mannie_worker"
rcvar="mannie_worker_enable"

load_rc_config "$name"

: "${{mannie_worker_enable:=NO}}"
: "${{mannie_worker_account:={user}}}"
: "${{mannie_worker_config:={config_file}}}"
: "${{mannie_worker_data_dir:={data_directory}}}"

pidfile="/var/run/${{name}}.pid"
procname="/usr/sbin/daemon"
command="/usr/sbin/daemon"
command_args="-f -r -R 10 -S -T ${{name}} -P ${{pidfile}} -u ${{mannie_worker_account}} {program} run --config ${{mannie_worker_config}}"
start_precmd="mannie_worker_prestart"

# Creates the data folder for the account, readable only by it.
mannie_worker_prestart()
{{
	install -d -o "${{mannie_worker_account}}" -m 0700 "${{mannie_worker_data_dir}}"
}}

run_rc_command "$1"
"#
    )
}

#[cfg(test)]
mod tests;
