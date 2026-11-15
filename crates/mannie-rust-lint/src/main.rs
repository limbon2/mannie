//! Rust source checks for the repository.

mod named_conditions;
mod statement_group_spacing;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use proc_macro2::LineColumn;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Status {
    Clean = 0,
    Findings = 1,
    Incomplete = 2,
}

#[derive(Clone, Copy)]
enum Rule {
    NamedConditions,
    StatementGroupSpacing,
}

impl Rule {
    fn name(self) -> &'static str {
        match self {
            Self::NamedConditions => "named-conditions",
            Self::StatementGroupSpacing => "statement-group-spacing",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::NamedConditions => {
                "Name this part of the compound condition while preserving short-circuit evaluation."
            }
            Self::StatementGroupSpacing => "Separate these statement groups with a blank line.",
        }
    }

    fn check(self, source: &str) -> syn::Result<Vec<LineColumn>> {
        match self {
            Self::NamedConditions => named_conditions::check(source),
            Self::StatementGroupSpacing => statement_group_spacing::check(source),
        }
    }
}

fn main() -> ExitCode {
    let mut output = io::stdout().lock();
    let mut errors = io::stderr().lock();
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    let status = run(&arguments, &mut output, &mut errors).unwrap_or(Status::Incomplete);
    ExitCode::from(status as u8)
}

fn run(
    arguments: &[OsString],
    output: &mut impl Write,
    errors: &mut impl Write,
) -> io::Result<Status> {
    if arguments
        .iter()
        .any(|arg| matches!(arg.to_str(), Some("--help" | "-h")))
    {
        writeln!(
            output,
            "Usage: mannie-rust-lint [--statement-group-spacing] [FILE ...]"
        )?;
        writeln!(
            output,
            "Without paths, scan Git-visible Rust sources under the current directory."
        )?;
        return Ok(Status::Clean);
    }

    let (rule, arguments) = match arguments.split_first() {
        Some((argument, paths)) if argument == "--statement-group-spacing" => {
            (Rule::StatementGroupSpacing, paths)
        }
        _ => (Rule::NamedConditions, arguments),
    };

    let discovered = arguments.is_empty();
    let paths = if discovered {
        match source_files() {
            Ok(paths) => paths,
            Err(error) => {
                writeln!(errors, "{}: {error}", rule.name())?;
                return Ok(Status::Incomplete);
            }
        }
    } else {
        arguments.iter().map(PathBuf::from).collect()
    };

    let mut result = Status::Clean;

    for path in paths {
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                // Git also lists tracked files that are deleted in the working tree.
                let removed_tracked_file = discovered && error.kind() == io::ErrorKind::NotFound;

                if removed_tracked_file {
                    continue;
                }
                writeln!(errors, "{}: {error}", path.display())?;
                result = Status::Incomplete;
                continue;
            }
        };

        result = result.max(check_file(rule, &path, &source, output, errors)?);
    }

    Ok(result)
}

fn source_files() -> Result<Vec<PathBuf>, String> {
    let files = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            "*.rs",
        ])
        .output()
        .map_err(|error| format!("could not list Rust sources: {error}"))?;

    if !files.status.success() {
        let detail = String::from_utf8_lossy(&files.stderr);
        return Err(format!("git ls-files failed: {}", detail.trim()));
    }

    let paths = String::from_utf8(files.stdout)
        .map_err(|error| format!("git ls-files returned a path outside UTF-8: {error}"))?;

    let unique_paths: BTreeSet<_> = paths
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .collect();
    Ok(unique_paths.into_iter().collect())
}

fn check_file(
    rule: Rule,
    path: &Path,
    source: &str,
    output: &mut impl Write,
    errors: &mut impl Write,
) -> io::Result<Status> {
    let findings = match rule.check(source) {
        Ok(findings) => findings,
        Err(error) => {
            let start = error.span().start();
            writeln!(
                errors,
                "{}:{}:{}: could not parse Rust source: {error}",
                path.display(),
                start.line,
                start.column + 1
            )?;
            return Ok(Status::Incomplete);
        }
    };

    for finding in &findings {
        writeln!(
            output,
            "{}:{}:{}: {}: {}",
            path.display(),
            finding.line,
            finding.column + 1,
            rule.name(),
            rule.message()
        )?;
    }

    if findings.is_empty() {
        Ok(Status::Clean)
    } else {
        Ok(Status::Findings)
    }
}
