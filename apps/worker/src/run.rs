//! The configuration that the commands work with (docs/features/workers/
//! service.md): `run` and `service run` refuse a configuration that the
//! worker's own account can change, and the commands for root or the
//! administrators read the installed one.

use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use mannie_types::clock::{Clock, SystemClock};

use crate::access;
use crate::cli::{ConfigArgument, RunArguments};
use crate::config::{self, Config};
use crate::paths::Folders;

/// The configuration file that a command names, or the system's.
pub(crate) fn config_file(argument: &ConfigArgument, folders: &Folders) -> PathBuf {
    argument
        .config
        .clone()
        .unwrap_or_else(|| folders.config_file())
}

/// The configuration that a worker runs with. A worker started with only a
/// data folder, as service definitions from before configurations start it,
/// runs with the default configuration when there is no file.
pub(crate) fn configuration(arguments: &RunArguments, folders: &Folders) -> anyhow::Result<Config> {
    let file = config_file(&arguments.config, folders);
    let data_dir = arguments.data_dir.as_deref().map(absolute).transpose()?;
    let missing = matches!(file.try_exists(), Ok(false));
    if let (true, None, Some(data_dir)) = (missing, &arguments.config.config, &data_dir) {
        let name = mannie_tools::SystemFacts::detect().host_facts().hostname;
        return Ok(config::without_file(&name, data_dir.clone(), folders));
    }
    let mut config = config::load(&file, folders, SystemClock.now())?;
    if let Some(data_dir) = data_dir {
        config.data_dir = data_dir;
    }
    let changeable = access::can_change(&file).with_context(|| {
        format!(
            "could not check whether this account can change the configuration {}",
            file.display()
        )
    })?;
    if changeable && !arguments.allow_writable_config {
        bail!(
            "this account can change the configuration {}, so the worker could widen its own ceiling. Make the file and its folder belong to root or the administrators, readable by this account; `--allow-writable-config` runs anyway, for a worker in a terminal or a test",
            file.display()
        );
    }
    Ok(config)
}

/// The installed configuration, for the commands that root or the
/// administrators run.
pub(crate) fn installed_configuration(
    argument: &ConfigArgument,
    folders: &Folders,
) -> anyhow::Result<Config> {
    let file = config_file(argument, folders);
    match config::load(&file, folders, SystemClock.now()) {
        Ok(config) => Ok(config),
        Err(config::ConfigError::Read { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            bail!(
                "there is no configuration at {}. Install the worker with `mannie-worker service install`, or name the configuration with --config",
                file.display()
            )
        }
        Err(error) => Err(error.into()),
    }
}

fn absolute(path: &Path) -> anyhow::Result<PathBuf> {
    std::path::absolute(path)
        .with_context(|| format!("the folder {} is not a usable path", path.display()))
}
