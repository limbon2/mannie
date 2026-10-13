//! What the systemd unit and the rc.d script have in common.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};

/// Whether `value` can stand in a unit file or a shell script without quoting.
/// Paths and account names with other characters are refused rather than
/// escaped, so no value can change the meaning of the service definition.
fn is_plain(value: &str) -> bool {
    let has_content = !value.is_empty();
    let only_plain_characters = value.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '/' | '.' | '_' | '-' | '+')
    });
    has_content && only_plain_characters
}

/// Checks a path that goes into the service definition.
pub(super) fn plain_path<'path>(path: &'path Path, what: &str) -> anyhow::Result<&'path str> {
    let text = path.to_str().filter(|text| is_plain(text));
    match text {
        Some(text) => Ok(text),
        None => bail!(
            "the path of the {what}, {}, has characters that the service definition cannot hold. Use a path of letters, digits and / . _ - +",
            path.display()
        ),
    }
}

/// Checks the account name that goes into the service definition, and that the account exists.
pub(super) fn existing_account(user: &str) -> anyhow::Result<()> {
    let starts_with_dash = user.starts_with('-');
    if starts_with_dash || !is_plain(user) || user.contains('/') {
        bail!("{user:?} is not an account name. Use the name of an account on this host");
    }
    let found = Command::new("id")
        .arg("-u")
        .arg(user)
        .output()
        .context("could not run id to look up the account")?;
    if !found.status.success() {
        bail!("the account {user} does not exist on this host");
    }
    Ok(())
}

/// Runs a system command and fails with its own error output when it fails.
pub(super) fn run(program: &str, arguments: &[&OsStr]) -> anyhow::Result<()> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .with_context(|| format!("could not run {program}"))?;
    if output.status.success() {
        return Ok(());
    }
    let shown: Vec<_> = arguments
        .iter()
        .map(|argument| argument.to_string_lossy())
        .collect();
    let error_output = String::from_utf8_lossy(&output.stderr);
    bail!(
        "`{program} {}` failed ({}): {}",
        shown.join(" "),
        output.status,
        error_output.trim()
    )
}

#[cfg(test)]
mod tests;
