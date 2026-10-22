//! Reads of a background command's output (docs/features/system-tools/
//! background-commands.md, reading output): from its log while it runs,
//! after the wait that the call asks for, and from its archive once it ended,
//! with the same line numbers. A read without a line moves the model's read
//! position past what it returned, and a read of a command that ended tells
//! the model its end.

use std::time::Duration;

use mannie_store::{Change, Durability};
use mannie_tools::{CommandRead, LiveLog, LogLines, ReadRequest, StopSignal};
use mannie_types::tools::ToolFailure;
use regex::Regex;

use super::{Tracked, ran_for, store_failure};
use crate::Runtime;

/// How a wait for a command's output ended.
#[derive(Debug, Default, PartialEq, Eq)]
struct Waited {
    /// The first line after the read position that matched the pattern.
    matched: Option<u64>,
    /// Whether the call was stopped first.
    stopped: bool,
}

/// Waits until the log ends, a line after the line `after` matches `until`,
/// the time passes, or the call is stopped. Without `until`, only the end,
/// the time and the stop end the wait.
async fn wait(
    log: &LiveLog,
    after: u64,
    duration: Duration,
    until: Option<&Regex>,
    stop: &StopSignal,
) -> Waited {
    let mut marks = log.subscribe();
    let deadline = tokio::time::Instant::now() + duration;
    let mut scanned = after;
    loop {
        // Lines that arrive after this mark change it again, so the next
        // round scans them.
        let mark = *marks.borrow_and_update();
        if let Some(pattern) = until
            && let Some(matched) = log.first_match_after(scanned, pattern)
        {
            return Waited {
                matched: Some(matched),
                stopped: false,
            };
        }
        scanned = scanned.max(mark.lines);
        if mark.ended || duration.is_zero() {
            return Waited::default();
        }
        tokio::select! {
            changed = marks.changed() => {
                if changed.is_err() {
                    return Waited::default();
                }
            }
            () = tokio::time::sleep_until(deadline) => return Waited::default(),
            () = stop.stopped() => {
                return Waited {
                    matched: None,
                    stopped: true,
                };
            }
        }
    }
}

/// The last line that a read covered, also among the lines that the log let
/// go.
fn last_line(lines: &LogLines) -> Option<u64> {
    let last_shown = lines.lines.last().map(|(number, _)| *number);
    let last_left_out = lines.left_out.map(|(_, last)| last);
    last_shown.max(last_left_out)
}

impl Runtime {
    /// Reads a command's output as the call asks.
    pub(super) async fn read_background(
        &self,
        request: &ReadRequest,
        stop: &StopSignal,
    ) -> Result<CommandRead, ToolFailure> {
        match self
            .inner
            .background
            .tracked(&request.conversation, request.number)
        {
            Some(tracked) => Ok(self.read_running(&tracked, request, stop).await),
            None => self.read_ended(request).await,
        }
    }

    /// Reads a command that runs, after the wait, from its log.
    async fn read_running(
        &self,
        tracked: &Tracked,
        request: &ReadRequest,
        stop: &StopSignal,
    ) -> CommandRead {
        let after = match request.from_line {
            Some(first) => first.saturating_sub(1),
            None => tracked.record.lock().await.read_up_to,
        };
        // The wait holds no lock, so the command's end is recorded meanwhile.
        let waited = wait(
            &tracked.log,
            after,
            request.wait,
            request.until.as_ref(),
            stop,
        )
        .await;
        let mut record = tracked.record.lock().await;
        let lines = match request.from_line {
            Some(first) => tracked.log.lines_from(first, request.max_lines),
            None => tracked.log.lines_after(record.read_up_to),
        };
        let total_lines = tracked.log.mark().lines;
        let mut changed = false;
        if request.from_line.is_none()
            && let Some(last) = last_line(&lines)
            && last > record.read_up_to
        {
            record.read_up_to = last;
            changed = true;
        }
        if record.ending.is_some() && !record.told {
            record.told = true;
            changed = true;
        }
        if changed {
            self.store_record(tracked, &record).await;
        }
        CommandRead {
            view: record.view(tracked.number, tracked.part_id.clone()),
            ran_for: ran_for(&record, self.inner.clock.now()),
            lines,
            total_lines,
            matched: waited.matched,
            stopped: waited.stopped,
            replaced: tracked.log.replaced(),
            lost: false,
        }
    }

    /// Reads a command that ended, from the archive of its log. A command
    /// that ran when Mannie ended unexpectedly has none: its output is lost.
    async fn read_ended(&self, request: &ReadRequest) -> Result<CommandRead, ToolFailure> {
        let store = &self.inner.store;
        let stored = store
            .background_command(&request.conversation, request.number)
            .await
            .map_err(|error| store_failure(&error, "read a background command"))?
            .ok_or(ToolFailure::CommandNotFound {
                number: request.number,
            })?;
        let mut document = stored.document;
        let archive = match &document.archive_id {
            Some(archive_id) => store
                .archive(&request.conversation, archive_id)
                .await
                .map_err(|error| {
                    store_failure(&error, "read the output of a background command")
                })?,
            None => None,
        };
        let lost = archive.is_none();
        let log = archive.map_or_else(LiveLog::new, |archive| {
            LiveLog::from_archive(&archive.content, document.head_lines, document.left_out)
        });
        let after = request
            .from_line
            .map_or(document.read_up_to, |first| first.saturating_sub(1));
        let lines = match request.from_line {
            Some(first) => log.lines_from(first, request.max_lines),
            None => log.lines_after(document.read_up_to),
        };
        let matched = request
            .until
            .as_ref()
            .and_then(|pattern| log.first_match_after(after, pattern));
        let mut changed = false;
        if request.from_line.is_none()
            && let Some(last) = last_line(&lines)
            && last > document.read_up_to
        {
            document.read_up_to = last;
            changed = true;
        }
        if document.ending.is_some() && !document.told {
            document.told = true;
            changed = true;
        }
        if changed {
            let change = Change::UpdateBackgroundCommand {
                number: request.number,
                document: Box::new(document.clone()),
                archive: None,
                announce: false,
            };
            let stored = self
                .inner
                .hubs
                .apply(
                    store,
                    &request.conversation,
                    vec![change],
                    Durability::Normal,
                )
                .await;
            if let Err(error) = stored {
                tracing::warn!(%error, error.code = error.code(), conversation = %request.conversation, number = request.number, "could not store the read position of a background command");
            }
        }
        let total_lines = if lost {
            document.lines
        } else {
            log.mark().lines
        };
        Ok(CommandRead {
            view: document.view(request.number, stored.part_id),
            ran_for: ran_for(&document, self.inner.clock.now()),
            lines,
            total_lines,
            matched,
            stopped: false,
            replaced: document.replaced,
            lost,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use mannie_tools::{LiveLog, LogLines, StopReason, StopSignal};
    use regex::Regex;

    use super::{Waited, last_line, wait};

    #[test]
    fn the_last_line_of_a_read_counts_the_lines_that_the_log_let_go() {
        let read = LogLines {
            lines: vec![(1, "a".to_owned())],
            left_out: Some((2, 40)),
        };

        assert_eq!(last_line(&read), Some(40));
        assert_eq!(last_line(&LogLines::default()), None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_wait_without_a_pattern_ends_at_the_time() {
        let log = LiveLog::new();

        let waited = wait(&log, 0, Duration::from_secs(30), None, &StopSignal::new()).await;

        assert_eq!(waited, Waited::default());
    }

    #[tokio::test(start_paused = true)]
    async fn a_wait_ends_at_the_end_of_the_log() {
        let log = LiveLog::new();
        let ending = log.clone();
        let ender = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            ending.end();
        });
        let started = tokio::time::Instant::now();

        let waited = wait(&log, 0, Duration::from_secs(600), None, &StopSignal::new()).await;

        ender.await.unwrap();
        assert_eq!(waited, Waited::default());
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[tokio::test(start_paused = true)]
    async fn a_stop_ends_a_wait() {
        let log = LiveLog::new();
        let stop = StopSignal::new();
        let stopper = stop.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            stopper.stop(StopReason::Stopped);
        });

        let waited = wait(&log, 0, Duration::from_secs(600), None, &stop).await;

        assert_eq!(
            waited,
            Waited {
                matched: None,
                stopped: true
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_line_that_was_written_before_the_wait_matches_at_once() {
        let log = LiveLog::from_archive("starting\nready on 5173\n", 2, None);
        let pattern = Regex::new("ready on \\d+").unwrap();

        let waited = wait(
            &log,
            0,
            Duration::from_secs(600),
            Some(&pattern),
            &StopSignal::new(),
        )
        .await;

        assert_eq!(waited.matched, Some(2));
    }
}
