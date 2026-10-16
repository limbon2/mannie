//! What the systemd unit and the rc.d script have in common.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};

use super::report;
use crate::paths::PROGRAM_LINK;

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

/// Runs a system command that answers a question with its exit status.
pub(super) fn succeeds(program: &str, arguments: &[&OsStr]) -> anyhow::Result<bool> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .with_context(|| format!("could not run {program}"))?;
    Ok(output.status.success())
}

/// Puts the link to the program on the search path, replacing an older link.
pub(super) fn link_program(program: &Path) -> anyhow::Result<()> {
    let link = Path::new(PROGRAM_LINK);
    match std::fs::symlink_metadata(link) {
        Ok(found) if found.file_type().is_symlink() => {
            std::fs::remove_file(link)
                .with_context(|| format!("could not replace the link {PROGRAM_LINK}"))?;
        }
        Ok(_file) => bail!(
            "{PROGRAM_LINK} is a file, not the link that `service install` makes. Move it away, then install again"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("could not look at {PROGRAM_LINK}"));
        }
    }
    std::os::unix::fs::symlink(program, link)
        .with_context(|| format!("could not link {PROGRAM_LINK} to {}", program.display()))?;
    report(&format!("Linked {PROGRAM_LINK} to {}.", program.display()));
    Ok(())
}

#[cfg(test)]
mod tests;
