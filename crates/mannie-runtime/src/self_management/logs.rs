//! Confined, bounded reader for Mannie's own redacted JSON and crash logs
//! (ADR-0081, T-131).
//!
//! Every file read is restricted strictly to regular files directly inside the
//! canonical `log_dir`. Path separators, `..`, control characters, symlinks,
//! and files outside `log_dir` are refused before opening.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default number of matching lines returned when `tail_lines` is omitted.
pub(crate) const DEFAULT_TAIL_LINES: u32 = 100;

/// Maximum number of lines returned in a single `mannie_read_logs` call.
pub(crate) const MAX_TAIL_LINES: u32 = 500;

/// Maximum raw bytes read from the tail of one log file (2 MiB) so reading a
/// full 32 MiB daily log stays bounded in time and memory.
const MAX_READ_BYTES: u64 = 2 * 1024 * 1024;

/// Maximum characters of a log file name accepted from the model.
const MAX_FILE_NAME_CHARS: usize = 256;

/// Arguments accepted by `mannie_read_logs`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LogReadRequest {
    /// Optional log file name inside Mannie's log folder. When omitted, reads
    /// the newest daily log file.
    pub(crate) file: Option<String>,
    /// Minimum log level filter: `trace`, `debug`, `info`, `warn`, or `error`.
    pub(crate) level: Option<String>,
    /// Optional prefix matched against the tracing `target` field (for example
    /// `mannie_ssh` or `mannie_mcp`).
    pub(crate) target_prefix: Option<String>,
    /// Optional case-insensitive substring filter applied to each raw line.
    pub(crate) contains: Option<String>,
    /// Maximum matching lines to return from the end of the file (1..=500,
    /// default 100).
    pub(crate) tail_lines: Option<u32>,
}

/// Metadata for one Mannie log file inside `log_dir`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LogFileInfo {
    /// File name inside `log_dir`.
    pub(crate) name: String,
    /// Either `"daily"` or `"crash"`.
    pub(crate) kind: &'static str,
    /// File size in bytes.
    pub(crate) bytes: u64,
    /// Last modification time in Unix milliseconds, if reported by the OS.
    pub(crate) modified_ms: Option<u64>,
}

/// Output produced by [`read_mannie_logs`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LogReadResult {
    /// Canonical path to Mannie's log directory.
    pub(crate) directory: String,
    /// Regular Mannie log files found in `directory`, newest first.
    pub(crate) files: Vec<LogFileInfo>,
    /// The file read, or `None` when no log files exist yet.
    pub(crate) selected_file: Option<String>,
    /// Matching log entries (parsed tracing JSON or `{ "raw": "..." }`).
    pub(crate) entries: Vec<Value>,
    /// Total matching lines seen inside the read window before tail/size limits.
    pub(crate) total_matching: u32,
    /// Whether older entries were dropped because of tail, window, or byte
    /// limits.
    pub(crate) truncated: bool,
}

/// Errors returned by [`read_mannie_logs`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LogReadError {
    /// Input parameter failed validation (invalid filename, traversal, level).
    Invalid(String),
    /// Requested file does not exist in `log_dir`.
    NotFound(String),
    /// I/O error reading the log directory or file.
    Io(String),
}

impl std::fmt::Display for LogReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) | Self::NotFound(message) | Self::Io(message) => {
                formatter.write_str(message)
            }
        }
    }
}

/// Lists regular Mannie log files in `log_dir`, newest first. Ignores symlinks
/// and non-log files.
pub(crate) fn list_log_files(log_dir: &Path) -> Result<Vec<LogFileInfo>, LogReadError> {
    if !log_dir.exists() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(log_dir)
        .map_err(|error| LogReadError::Io(format!("could not read log directory: {error}")))?;
    let mut files = Vec::new();

    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(meta) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !is_mannie_log_name(&name) {
            continue;
        }

        let bytes = meta.len();
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .and_then(|duration| u64::try_from(duration.as_millis()).ok());

        files.push(LogFileInfo {
            kind: classify_log_kind(&name),
            name,
            bytes,
            modified_ms,
        });
    }

    files.sort_by(|left, right| {
        right
            .modified_ms
            .cmp(&left.modified_ms)
            .then_with(|| right.name.cmp(&left.name))
    });
    Ok(files)
}

/// Reads bounded, filtered log entries strictly from `log_dir`.
pub(crate) fn read_mannie_logs(
    log_dir: &Path,
    request: &LogReadRequest,
    max_output_bytes: usize,
) -> Result<LogReadResult, LogReadError> {
    let filters = LogFilters::from_request(request)?;

    if let Some(requested) = request.file.as_deref() {
        validate_file_name(requested)?;
    }

    if !log_dir.exists() {
        return Ok(empty_log_result(log_dir.display().to_string(), Vec::new()));
    }

    let canonical_dir = fs::canonicalize(log_dir).map_err(|error| {
        LogReadError::Io(format!("could not resolve log directory path: {error}"))
    })?;
    let files = list_log_files(&canonical_dir)?;

    let Some(selected_name) = select_log_file(&files, request.file.as_deref())? else {
        return Ok(empty_log_result(canonical_dir.display().to_string(), files));
    };

    let canonical_file = resolve_confined_log(&canonical_dir, &selected_name)?;
    let (raw_text, window_truncated) = read_tail_text(&canonical_file)?;
    let matching = filter_log_lines(&raw_text, &filters);
    let total_matching = u32::try_from(matching.len()).unwrap_or(u32::MAX);
    let (entries, truncated) = cap_log_entries(
        matching,
        filters.tail_lines,
        max_output_bytes,
        window_truncated,
    );

    Ok(LogReadResult {
        directory: canonical_dir.display().to_string(),
        files,
        selected_file: Some(selected_name),
        entries,
        total_matching,
        truncated,
    })
}

struct LogFilters<'a> {
    min_level: Option<u8>,
    tail_lines: u32,
    target_prefix: Option<&'a str>,
    contains: Option<String>,
}

impl<'a> LogFilters<'a> {
    fn from_request(request: &'a LogReadRequest) -> Result<Self, LogReadError> {
        let min_level = match request.level.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(text) => Some(parse_level(text)?),
        };
        let tail_lines = request
            .tail_lines
            .unwrap_or(DEFAULT_TAIL_LINES)
            .clamp(1, MAX_TAIL_LINES);
        let target_prefix = request
            .target_prefix
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty());
        let contains = request
            .contains
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_ascii_lowercase);
        Ok(Self {
            min_level,
            tail_lines,
            target_prefix,
            contains,
        })
    }
}

fn empty_log_result(directory: String, files: Vec<LogFileInfo>) -> LogReadResult {
    LogReadResult {
        directory,
        files,
        selected_file: None,
        entries: Vec::new(),
        total_matching: 0,
        truncated: false,
    }
}

fn select_log_file(
    files: &[LogFileInfo],
    requested: Option<&str>,
) -> Result<Option<String>, LogReadError> {
    match requested {
        Some(name) => {
            if !files.iter().any(|file| file.name == name) {
                return Err(LogReadError::NotFound(format!(
                    "log file {name:?} was not found in Mannie's log directory"
                )));
            }
            Ok(Some(name.to_owned()))
        }
        None => Ok(files
            .iter()
            .find(|file| file.kind == "daily")
            .or_else(|| files.first())
            .map(|file| file.name.clone())),
    }
}

fn resolve_confined_log(
    canonical_dir: &Path,
    selected_name: &str,
) -> Result<std::path::PathBuf, LogReadError> {
    let candidate = canonical_dir.join(selected_name);
    let symlink_meta = fs::symlink_metadata(&candidate)
        .map_err(|error| LogReadError::Io(format!("could not stat log file: {error}")))?;
    if !symlink_meta.is_file() {
        return Err(LogReadError::Invalid(format!(
            "log path {selected_name:?} is not a regular file"
        )));
    }

    let canonical_file = fs::canonicalize(&candidate)
        .map_err(|error| LogReadError::Io(format!("could not resolve log file: {error}")))?;
    if canonical_file.parent() != Some(canonical_dir) {
        return Err(LogReadError::Invalid(
            "log path resolved outside Mannie's log directory".to_owned(),
        ));
    }
    Ok(canonical_file)
}

fn filter_log_lines(raw_text: &str, filters: &LogFilters<'_>) -> Vec<Value> {
    let mut matching = Vec::new();
    for line in raw_text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(needle) = &filters.contains
            && !trimmed.to_ascii_lowercase().contains(needle)
        {
            continue;
        }

        let parsed = serde_json::from_str::<Value>(trimmed)
            .ok()
            .filter(Value::is_object);

        if let Some(min_level) = filters.min_level {
            let line_level = parsed
                .as_ref()
                .and_then(|value| value.get("level"))
                .and_then(Value::as_str)
                .and_then(|level| parse_level(level).ok());
            if line_level.is_none_or(|level| level < min_level) {
                continue;
            }
        }

        if let Some(prefix) = filters.target_prefix {
            let matches_target = parsed
                .as_ref()
                .and_then(|value| value.get("target"))
                .and_then(Value::as_str)
                .is_some_and(|target| target.starts_with(prefix));
            if !matches_target {
                continue;
            }
        }

        let entry = parsed.unwrap_or_else(|| serde_json::json!({ "raw": trimmed }));
        matching.push(entry);
    }
    matching
}

fn cap_log_entries(
    mut matching: Vec<Value>,
    tail_lines: u32,
    max_output_bytes: usize,
    window_truncated: bool,
) -> (Vec<Value>, bool) {
    let tail_cap = usize::try_from(tail_lines).unwrap_or(usize::MAX);
    let mut truncated = window_truncated;

    if matching.len() > tail_cap {
        let drop_count = matching.len() - tail_cap;
        matching.drain(..drop_count);
        truncated = true;
    }

    let byte_budget = max_output_bytes.saturating_sub(2048).max(1024);
    let mut kept_reversed = Vec::with_capacity(matching.len());
    let mut used_bytes = 0usize;

    for entry in matching.into_iter().rev() {
        let entry_bytes = serde_json::to_string(&entry).map_or(128, |text| text.len() + 2);
        if !kept_reversed.is_empty() && used_bytes.saturating_add(entry_bytes) > byte_budget {
            truncated = true;
            break;
        }
        used_bytes = used_bytes.saturating_add(entry_bytes);
        kept_reversed.push(entry);
    }
    kept_reversed.reverse();
    (kept_reversed, truncated)
}

fn validate_file_name(name: &str) -> Result<(), LogReadError> {
    if name.is_empty() || name.chars().count() > MAX_FILE_NAME_CHARS {
        return Err(LogReadError::Invalid(
            "log file name must be between 1 and 256 characters".to_owned(),
        ));
    }
    if name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name.contains(':')
        || name.chars().any(char::is_control)
    {
        return Err(LogReadError::Invalid(
            "log file name must be a plain file name inside Mannie's log directory".to_owned(),
        ));
    }
    if !is_mannie_log_name(name) {
        return Err(LogReadError::Invalid(format!(
            "{name:?} is not a Mannie log file"
        )));
    }
    Ok(())
}

fn is_mannie_log_name(name: &str) -> bool {
    name.starts_with("mannie")
        && Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("log"))
}

fn classify_log_kind(name: &str) -> &'static str {
    if name.contains("crash") {
        "crash"
    } else {
        "daily"
    }
}

fn parse_level(text: &str) -> Result<u8, LogReadError> {
    match text.to_ascii_lowercase().as_str() {
        "trace" => Ok(0),
        "debug" => Ok(1),
        "info" => Ok(2),
        "warn" | "warning" => Ok(3),
        "error" => Ok(4),
        _ => Err(LogReadError::Invalid(format!(
            "invalid log level {text:?}; expected trace, debug, info, warn, or error"
        ))),
    }
}

fn read_tail_text(path: &Path) -> Result<(String, bool), LogReadError> {
    let mut file = File::open(path)
        .map_err(|error| LogReadError::Io(format!("could not open log file: {error}")))?;
    let length = file
        .metadata()
        .map_err(|error| LogReadError::Io(format!("could not read log metadata: {error}")))?
        .len();

    let (truncated, to_read) = if length > MAX_READ_BYTES {
        file.seek(SeekFrom::End(
            -i64::try_from(MAX_READ_BYTES).unwrap_or(i64::MAX),
        ))
        .map_err(|error| LogReadError::Io(format!("could not seek log file: {error}")))?;
        (true, usize::try_from(MAX_READ_BYTES).unwrap_or(usize::MAX))
    } else {
        (false, usize::try_from(length).unwrap_or(usize::MAX))
    };

    let mut buffer = Vec::with_capacity(to_read);
    file.take(MAX_READ_BYTES)
        .read_to_end(&mut buffer)
        .map_err(|error| LogReadError::Io(format!("could not read log file: {error}")))?;

    let text = String::from_utf8_lossy(&buffer);
    if truncated && let Some(newline) = text.find('\n') {
        return Ok((text[newline + 1..].to_owned(), true));
    }
    Ok((text.into_owned(), truncated))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{LogReadError, LogReadRequest, read_mannie_logs};

    #[test]
    fn filters_daily_json_logs_by_level_target_and_substring() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("mannie.2026-11-26.log");
        fs::write(
            &file,
            concat!(
                r#"{"timestamp":"2026-11-26T01:00:00Z","level":"INFO","target":"mannie_runtime","fields":{"message":"started"}}"#,
                "\n",
                r#"{"timestamp":"2026-11-26T01:01:00Z","level":"WARN","target":"mannie_ssh::connect","fields":{"message":"host key mismatch"}}"#,
                "\n",
                r#"{"timestamp":"2026-11-26T01:02:00Z","level":"ERROR","target":"mannie_ssh::connect","fields":{"message":"connection refused on port 2222"}}"#,
                "\n",
                r#"{"timestamp":"2026-11-26T01:03:00Z","level":"ERROR","target":"mannie_mcp::client","fields":{"message":"stdio exit"}}"#,
                "\n",
            ),
        )
        .unwrap();

        let result = read_mannie_logs(
            dir.path(),
            &LogReadRequest {
                file: None,
                level: Some("warn".to_owned()),
                target_prefix: Some("mannie_ssh".to_owned()),
                contains: Some("port 2222".to_owned()),
                tail_lines: Some(50),
            },
            32 * 1024,
        )
        .unwrap();

        assert_eq!(
            result.selected_file.as_deref(),
            Some("mannie.2026-11-26.log")
        );
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0]["target"], "mannie_ssh::connect");
        assert!(!result.truncated);
    }

    #[test]
    fn refuses_traversal_separators_and_non_log_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("mannie.2026-11-26.log"), "{}\n").unwrap();
        fs::write(dir.path().join("secret.txt"), "nope\n").unwrap();

        for bad in [
            "../mannie.sqlite",
            "..\\mannie.sqlite",
            "sub/mannie.log",
            "C:\\Windows\\win.ini",
            "secret.txt",
            "mannie\0.log",
        ] {
            let error = read_mannie_logs(
                dir.path(),
                &LogReadRequest {
                    file: Some(bad.to_owned()),
                    ..LogReadRequest::default()
                },
                32 * 1024,
            )
            .unwrap_err();
            assert!(
                matches!(error, LogReadError::Invalid(_)),
                "{bad}: {error:?}"
            );
        }
    }
}
