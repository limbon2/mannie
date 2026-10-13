//! Crash files: a panic is a bug, and the panic hook records it, redacted, in
//! the log and in a crash file of its own, written before the process can end.

use std::backtrace::Backtrace;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mannie_types::clock::Clock;
use time::OffsetDateTime;
use time::macros::format_description;

use crate::redact::Redactor;

/// Installs the panic hook. It replaces the default hook, whose output would
/// not be redacted, and prints the redacted report to standard error when
/// `console` is set.
pub(crate) fn install_panic_hook(
    directory: PathBuf,
    redactor: Redactor,
    clock: Arc<dyn Clock>,
    console: bool,
) {
    std::panic::set_hook(Box::new(move |info| {
        let report = redactor.redact(&crash_report(info)).into_owned();
        let message = redactor.redact(&panic_message(info)).into_owned();
        let location = info.location().map(ToString::to_string).unwrap_or_default();
        tracing::error!(
            target: "panic",
            %location,
            panic_message = %message,
            "a panic stopped a thread; see the crash file"
        );
        let written = write_crash_file(&directory, clock.now().as_millis(), &report);
        if console {
            #[expect(
                clippy::print_stderr,
                reason = "the console shows crash reports in development builds"
            )]
            {
                eprintln!("{report}");
                if let Err(error) = &written {
                    eprintln!("could not write the crash file: {error}");
                }
            }
        }
    }));
}

/// The panic's message, when it has one.
fn panic_message(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "the panic carried no message".to_owned()
    }
}

fn crash_report(info: &PanicHookInfo<'_>) -> String {
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("unnamed");
    let location = info
        .location()
        .map_or_else(|| "unknown".to_owned(), ToString::to_string);
    format!(
        "Mannie {} stopped a thread because of a bug.\n\nmessage: {}\nlocation: {location}\nthread: {thread_name}\n\nbacktrace:\n{}\n",
        env!("CARGO_PKG_VERSION"),
        panic_message(info),
        Backtrace::force_capture(),
    )
}

/// Writes the report to `crash-<UTC time>.txt` in the log folder. A file
/// that already exists is never overwritten.
fn write_crash_file(directory: &Path, millis: i64, report: &str) -> std::io::Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let format =
        format_description!("[year][month][day]T[hour][minute][second].[subsecond digits:3]Z");
    let stamp = OffsetDateTime::from_unix_timestamp_nanos(i128::from(millis) * 1_000_000)
        .ok()
        .and_then(|moment| moment.format(&format).ok())
        .unwrap_or_else(|| millis.to_string());
    for attempt in 0_u32..100 {
        let name = if attempt == 0 {
            format!("crash-{stamp}.txt")
        } else {
            format!("crash-{stamp}-{attempt}.txt")
        };
        let path = directory.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(report.as_bytes())?;
                file.sync_all()?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "too many crash files with the same time",
    ))
}
