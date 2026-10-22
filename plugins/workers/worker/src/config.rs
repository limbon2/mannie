//! The worker's configuration (plugins/workers/docs/service.md, the
//! configuration): one versioned TOML file that root or the administrators
//! own. Every error names the key and what is wrong with it, and a newer
//! version or an unknown key is refused, so a worker never misreads a file.

use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use mannie_host_kit::tools::Ceiling;
use mannie_host_kit::types::CommandRule;
use mannie_host_kit::types::RuleId;
use mannie_plugin_sdk::contracts::CeilingMode;
use mannie_plugin_sdk::contracts::UtcMillis;
use serde::Deserialize;

use crate::paths::{Folders, HOST_INSTRUCTIONS_FILE};

/// The version of the configuration that this worker reads.
pub(crate) const VERSION: i64 = 1;

/// The address the worker listens on when the configuration names none.
pub(crate) const DEFAULT_LISTEN: &str = "0.0.0.0:7445";

/// The most characters of a worker's name.
const MAX_NAME_CHARS: usize = 64;

/// The most characters of a worker's purpose.
const MAX_PURPOSE_CHARS: usize = 1_000;

/// How many tasks run at once when the configuration names no number, and
/// the range it may name.
const DEFAULT_MAX_PARALLEL_TASKS: u32 = 4;
const MAX_PARALLEL_TASKS: u32 = 16;

/// Why the configuration cannot be used.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ConfigError {
    /// The file could not be read.
    #[error("could not read the configuration {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// The file is not TOML, has an unknown key or misses a required one.
    #[error("the configuration {path} is not valid: {message}")]
    Syntax { path: PathBuf, message: String },
    /// A newer worker wrote the file.
    #[error(
        "the configuration {path} has version {version}, which a newer Mannie worker reads; this worker reads version {VERSION}"
    )]
    NewerVersion { path: PathBuf, version: i64 },
    /// A value is not allowed.
    #[error("in the configuration {path}, `{key}` {problem}")]
    Invalid {
        path: PathBuf,
        key: String,
        problem: String,
    },
}

/// The configuration as the file holds it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    // `parse` reads the version first; the key is here so that it is known.
    #[serde(rename = "version")]
    _version: i64,
    name: String,
    #[serde(default)]
    purpose: String,
    listen: Option<String>,
    data_dir: Option<PathBuf>,
    host_instructions: Option<PathBuf>,
    max_parallel_tasks: Option<i64>,
    #[serde(default)]
    ceiling: CeilingFile,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CeilingFile {
    #[serde(default)]
    mode: CeilingMode,
    #[serde(default)]
    allowed_folders: Vec<PathBuf>,
    #[serde(default)]
    rules: Vec<RuleFile>,
    allow_full_access_from_master: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    command: String,
    folder: Option<PathBuf>,
}

/// A configuration that the worker can run with.
#[derive(Debug, Clone)]
pub(crate) struct Config {
    /// The file it came from, if any.
    pub(crate) file: Option<PathBuf>,
    /// The worker's name.
    pub(crate) name: String,
    /// What the host is for.
    pub(crate) purpose: String,
    /// The address and port to listen on.
    pub(crate) listen: SocketAddr,
    /// The data folder.
    pub(crate) data_dir: PathBuf,
    /// The host instructions file, which may not exist.
    pub(crate) host_instructions: PathBuf,
    /// How many tasks run at once.
    pub(crate) max_parallel_tasks: u32,
    /// The ceiling of every tool call on the host.
    pub(crate) ceiling: Ceiling,
    /// Whether a conversation in full access on the master may answer the
    /// host's requests (ADR-0029, decision 5). True when the file does not
    /// say.
    pub(crate) accepts_master_full_access: bool,
}

/// Reads and checks the configuration in `file`. `now` dates the host's
/// rules.
pub(crate) fn load(file: &Path, folders: &Folders, now: UtcMillis) -> Result<Config, ConfigError> {
    let text = fs::read_to_string(file).map_err(|source| ConfigError::Read {
        path: file.to_path_buf(),
        source,
    })?;
    parse(&text, file, folders, now)
}

/// The configuration of a worker that runs without a file: a name from the
/// host, the default listen address, and an `ask` ceiling without allowed
/// folders. Service definitions from before configurations ran this way.
pub(crate) fn without_file(name: &str, data_dir: PathBuf, folders: &Folders) -> Config {
    Config {
        file: None,
        name: name.to_owned(),
        purpose: String::new(),
        listen: DEFAULT_LISTEN
            .parse()
            .expect("the default listen address is an address and port"),
        data_dir,
        host_instructions: folders.config.join(HOST_INSTRUCTIONS_FILE),
        max_parallel_tasks: DEFAULT_MAX_PARALLEL_TASKS,
        ceiling: Ceiling {
            mode: CeilingMode::Ask,
            allowed_folders: Vec::new(),
            rules: Vec::new(),
        },
        accepts_master_full_access: true,
    }
}

/// Checks the configuration `text` of `file`.
pub(crate) fn parse(
    text: &str,
    file: &Path,
    folders: &Folders,
    now: UtcMillis,
) -> Result<Config, ConfigError> {
    // The version is read first, so that a newer file is refused for its
    // version, before one of its new keys looks unknown.
    let table: toml::Table = toml::from_str(text).map_err(|error| ConfigError::Syntax {
        path: file.to_path_buf(),
        message: error.to_string(),
    })?;
    let invalid = |key: &str, problem: String| ConfigError::Invalid {
        path: file.to_path_buf(),
        key: key.to_owned(),
        problem,
    };
    match table.get("version").map(toml::Value::as_integer) {
        None => {
            return Err(invalid(
                "version",
                format!("is missing; write `version = {VERSION}`"),
            ));
        }
        Some(None) => return Err(invalid("version", "is not a number".to_owned())),
        Some(Some(version)) if version > VERSION => {
            return Err(ConfigError::NewerVersion {
                path: file.to_path_buf(),
                version,
            });
        }
        Some(Some(version)) if version < VERSION => {
            return Err(invalid(
                "version",
                format!("is {version}; the first version is {VERSION}"),
            ));
        }
        Some(Some(_current)) => {}
    }
    let raw: ConfigFile = toml::from_str(text).map_err(|error| ConfigError::Syntax {
        path: file.to_path_buf(),
        message: error.to_string(),
    })?;
    let config_folder = file.parent().unwrap_or(&folders.config);
    let accepts_master_full_access = raw.ceiling.allow_full_access_from_master.unwrap_or(true);
    Ok(Config {
        file: Some(file.to_path_buf()),
        name: checked_name(&raw.name).map_err(|problem| invalid("name", problem))?,
        purpose: checked_purpose(&raw.purpose).map_err(|problem| invalid("purpose", problem))?,
        listen: checked_listen(raw.listen.as_deref())
            .map_err(|problem| invalid("listen", problem))?,
        data_dir: match raw.data_dir {
            Some(folder) => absolute(folder).map_err(|problem| invalid("data_dir", problem))?,
            None => folders.data.clone(),
        },
        host_instructions: match raw.host_instructions {
            Some(path) => {
                absolute(path).map_err(|problem| invalid("host_instructions", problem))?
            }
            None => config_folder.join(HOST_INSTRUCTIONS_FILE),
        },
        max_parallel_tasks: checked_task_limit(raw.max_parallel_tasks)
            .map_err(|problem| invalid("max_parallel_tasks", problem))?,
        ceiling: checked_ceiling(raw.ceiling, now)
            .map_err(|(key, problem)| invalid(&key, problem))?,
        accepts_master_full_access,
    })
}

fn checked_name(name: &str) -> Result<String, String> {
    let count = name.chars().count();
    if name.trim().is_empty() {
        return Err("is empty; name the worker, such as `name = \"build-01\"`".to_owned());
    }
    if count > MAX_NAME_CHARS {
        return Err(format!(
            "has {count} characters; the most is {MAX_NAME_CHARS}"
        ));
    }
    if name.chars().any(char::is_control) {
        return Err("has a control character".to_owned());
    }
    Ok(name.to_owned())
}

fn checked_purpose(purpose: &str) -> Result<String, String> {
    let count = purpose.chars().count();
    if count > MAX_PURPOSE_CHARS {
        return Err(format!(
            "has {count} characters; the most is {MAX_PURPOSE_CHARS}"
        ));
    }
    let has_control = purpose
        .chars()
        .any(|character| character.is_control() && character != '\n');
    if has_control {
        return Err("has a control character".to_owned());
    }
    Ok(purpose.to_owned())
}

fn checked_listen(listen: Option<&str>) -> Result<SocketAddr, String> {
    let text = listen.unwrap_or(DEFAULT_LISTEN);
    text.parse().map_err(|_invalid| {
        format!("is {text:?}, which is not an address and a port, such as {DEFAULT_LISTEN:?}")
    })
}

fn checked_task_limit(limit: Option<i64>) -> Result<u32, String> {
    let Some(limit) = limit else {
        return Ok(DEFAULT_MAX_PARALLEL_TASKS);
    };
    u32::try_from(limit)
        .ok()
        .filter(|limit| (1..=MAX_PARALLEL_TASKS).contains(limit))
        .ok_or_else(|| format!("is {limit}; it is from 1 to {MAX_PARALLEL_TASKS}"))
}

fn absolute(path: PathBuf) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!(
            "is {}, which is not an absolute path",
            path.display()
        ))
    }
}

/// An absolute folder that exists, in the canonical form that the policy
/// compares.
fn existing_folder(path: PathBuf) -> Result<String, String> {
    let path = absolute(path)?;
    mannie_host_kit::tools::canonical_folder(&path).map_err(|_missing| {
        format!(
            "names {}, which is not a folder that exists on this host",
            path.display()
        )
    })
}

fn checked_ceiling(ceiling: CeilingFile, now: UtcMillis) -> Result<Ceiling, (String, String)> {
    let mut allowed_folders = Vec::new();
    for (index, folder) in ceiling.allowed_folders.into_iter().enumerate() {
        let canonical = existing_folder(folder)
            .map_err(|problem| (format!("ceiling.allowed_folders[{index}]"), problem))?;
        allowed_folders.push(PathBuf::from(canonical));
    }
    let mut rules = Vec::new();
    for (index, rule) in ceiling.rules.into_iter().enumerate() {
        let words = mannie_host_kit::tools::command_words(&rule.command).ok_or_else(|| {
            (
                format!("ceiling.rules[{index}].command"),
                format!(
                    "is {:?}, which a rule cannot cover: it needs whole words without ; & | $ ` ( ) {{ }} < > or line breaks",
                    rule.command
                ),
            )
        })?;
        let folder = rule
            .folder
            .map(existing_folder)
            .transpose()
            .map_err(|problem| (format!("ceiling.rules[{index}].folder"), problem))?;
        // A host rule allows its commands in both modes for as long as their
        // task runs, so it needs no mode of its own (ADR-0041, decision 9).
        rules.push(CommandRule {
            id: new_rule_id(),
            words,
            folder,
            created_at: now,
            background: false,
        });
    }
    Ok(Ceiling {
        mode: ceiling.mode,
        allowed_folders,
        rules,
    })
}

/// A host rule's ID, which only this run of the worker uses.
fn new_rule_id() -> RuleId {
    RuleId::parse(&uuid::Uuid::now_v7().to_string())
        .expect("the uuid crate writes UUIDs in their hyphenated form")
}

/// The configuration that `service install` writes when there is none, for
/// a worker named `name`.
pub(crate) fn initial_file(name: &str, folders: &Folders) -> String {
    let name = if is_plain_name(name) {
        name
    } else {
        "mannie-worker"
    };
    format!(
        "\
# The Mannie worker's configuration. Only root or the administrators can
# change it. Restart the service after a change.
version = 1

# The worker's name, which Mannie shows and its assistant uses.
name = \"{name}\"

# What this host is for, in a sentence or two, for Mannie's assistant.
purpose = \"\"

# The address and port to listen on.
# listen = \"{DEFAULT_LISTEN}\"

# The data folder.
# data_dir = {data}

# How many tasks run at once, from 1 to {MAX_PARALLEL_TASKS}.
# max_parallel_tasks = {DEFAULT_MAX_PARALLEL_TASKS}

[ceiling]
# read_only: the worker only reads and searches.
# ask: it asks you, through Mannie, before every change and command.
# autonomous: it changes files in the allowed folders, and runs the commands
# that the rules allow, without asking; it asks for everything else.
# full_access: it changes files and runs commands anywhere on this host,
# its own folders included, without asking anybody.
mode = \"ask\"

# Whether a conversation in full access in Mannie answers this host's
# requests without asking you. Set it to false to keep deciding yourself.
# allow_full_access_from_master = true

# Absolute folders where the worker reads without asking, and changes files
# in autonomous mode. Give the worker's account access to them.
allowed_folders = []

# Commands that run without asking in autonomous mode, by their first whole
# words, such as {{ command = \"git status\" }}, optionally only in a folder:
# {{ command = \"cargo test\", folder = \"/srv/app\" }}.
rules = []
",
        data = toml_literal(&folders.data.display().to_string()),
    )
}

/// Whether a name can stand in the initial file as it is.
fn is_plain_name(name: &str) -> bool {
    let fits = !name.is_empty() && name.chars().count() <= MAX_NAME_CHARS;
    fits && name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_'))
}

/// A TOML literal string, which keeps backslashes as they are.
fn toml_literal(text: &str) -> String {
    if text.contains('\'') {
        format!("{text:?}")
    } else {
        format!("'{text}'")
    }
}

#[cfg(test)]
mod tests;
