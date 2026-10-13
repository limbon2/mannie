//! The worker as an rc.d service. daemon(8) runs it under the account,
//! restarts it when it exits, and sends its standard output to syslog.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use anyhow::{Context, bail};

use super::{report, unix};

/// The rc.d name, a shell variable name.
const RC_NAME: &str = "mannie_worker";

/// Where `install` writes the rc.d script.
const SCRIPT_PATH: &str = "/usr/local/etc/rc.d/mannie_worker";

pub(super) fn install(executable: &Path, data_directory: &Path, user: &str) -> anyhow::Result<()> {
    let executable = unix::plain_path(executable, "worker's executable")?;
    let data_directory = unix::plain_path(data_directory, "data folder")?;
    unix::existing_account(user)?;
    write_script(&rc_script(executable, data_directory, user))?;
    report(&format!("Wrote the rc.d script {SCRIPT_PATH}."));
    unix::run(
        "/usr/sbin/sysrc",
        &[format!("{RC_NAME}_enable=YES").as_ref()],
    )?;
    unix::run("/usr/sbin/service", &[RC_NAME.as_ref(), "start".as_ref()]).with_context(|| {
        format!("the service is installed but did not start. `service {RC_NAME} status` and /var/log/messages show why")
    })?;
    report(&format!(
        "Started the service {RC_NAME} as {user}. It starts with the system."
    ));
    Ok(())
}

pub(super) fn uninstall() -> anyhow::Result<()> {
    if !Path::new(SCRIPT_PATH).exists() {
        bail!("the service {RC_NAME} is not installed: {SCRIPT_PATH} does not exist");
    }
    let running = unix::run(
        "/usr/sbin/service",
        &[RC_NAME.as_ref(), "onestatus".as_ref()],
    )
    .is_ok();
    if running {
        unix::run("/usr/sbin/service", &[RC_NAME.as_ref(), "onestop".as_ref()])?;
        report(&format!("Stopped the service {RC_NAME}."));
    }
    unix::run(
        "/usr/sbin/sysrc",
        &["-x".as_ref(), format!("{RC_NAME}_enable").as_ref()],
    )?;
    fs::remove_file(SCRIPT_PATH).with_context(|| format!("could not remove {SCRIPT_PATH}"))?;
    report(&format!(
        "Removed the rc.d script {SCRIPT_PATH}. The data folder was kept."
    ));
    Ok(())
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
/// needs quoting. rc.conf can override the account and the data folder.
fn rc_script(executable: &str, data_directory: &str, user: &str) -> String {
    format!(
        r#"#!/bin/sh
# Written by `mannie-worker service install`; `mannie-worker service uninstall` removes it.
#
# PROVIDE: mannie_worker
# REQUIRE: LOGIN NETWORKING
# KEYWORD: shutdown
#
# mannie_worker_enable (bool):     Start the worker with the system. Default: NO.
# mannie_worker_user (string):     The account that runs the worker.
# mannie_worker_data_dir (string): The worker's data folder.

. /etc/rc.subr

name="mannie_worker"
rcvar="mannie_worker_enable"

load_rc_config "$name"

: "${{mannie_worker_enable:=NO}}"
: "${{mannie_worker_user:={user}}}"
: "${{mannie_worker_data_dir:={data_directory}}}"

pidfile="/var/run/${{name}}.pid"
procname="/usr/sbin/daemon"
command="/usr/sbin/daemon"
command_args="-r -R 10 -S -T ${{name}} -P ${{pidfile}} -u ${{mannie_worker_user}} {executable} run --data-dir ${{mannie_worker_data_dir}}"
start_precmd="mannie_worker_prestart"

# Creates the data folder for the account, readable only by it.
mannie_worker_prestart()
{{
	install -d -o "${{mannie_worker_user}}" -m 0700 "${{mannie_worker_data_dir}}"
}}

run_rc_command "$1"
"#
    )
}

#[cfg(test)]
mod tests;
