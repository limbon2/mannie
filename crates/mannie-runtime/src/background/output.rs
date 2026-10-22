//! The owner's view of a background command (docs/features/system-tools/
//! background-commands.md, interface): Stop from the toolbar, the panel or
//! the call's card, and the output as it arrives, from the lines after the
//! line that the view holds, or its last lines, until the command ended.

use std::collections::VecDeque;
use std::sync::Arc;

use mannie_tools::{LiveLog, LogLines, LogMark};
use mannie_types::ids::ConversationId;
use mannie_types::tools::{
    BackgroundCommandView, BackgroundEnding, CommandOutputUpdate, MAX_OUTPUT_VIEW_LINES,
    StopBackgroundCommand, SubscribeCommandOutput, ToolFailure, ToolsError,
};
use tokio::sync::watch;

use super::Tracked;
use crate::Runtime;

/// The most lines of one update, so a long output reaches the view in
/// several frames.
const UPDATE_LINES: usize = 500;

fn line_number(number: u64) -> u32 {
    u32::try_from(number).unwrap_or(u32::MAX)
}

/// Where a view's command is: in the registry while it runs, with its view
/// once it ended.
#[derive(Debug)]
enum Followed {
    Running(Arc<Tracked>),
    Ended(Box<BackgroundCommandView>),
}

impl Followed {
    async fn view(&self) -> BackgroundCommandView {
        match self {
            Self::Running(tracked) => {
                let record = tracked.record.lock().await;
                record.view(tracked.number, tracked.part_id.clone())
            }
            Self::Ended(view) => (**view).clone(),
        }
    }

    /// The command once its end is recorded.
    async fn ended(&self) -> BackgroundCommandView {
        if let Self::Running(tracked) = self {
            tracked.wait_ended().await;
        }
        self.view().await
    }
}

/// A view of a command's output: the command, the lines that the view asked
/// for, new lines as they arrive, and the command as it ended.
#[derive(Debug)]
pub struct CommandOutput {
    followed: Followed,
    log: LiveLog,
    marks: watch::Receiver<LogMark>,
    /// The last line that the view received, or that it holds.
    sent: u64,
    pending: VecDeque<CommandOutputUpdate>,
    started: bool,
    done: bool,
}

impl CommandOutput {
    fn new(followed: Followed, log: LiveLog, sent: u64) -> Self {
        let marks = log.subscribe();
        Self {
            followed,
            log,
            marks,
            sent,
            pending: VecDeque::new(),
            started: false,
            done: false,
        }
    }

    /// The next update. After [`CommandOutputUpdate::Ended`], there are no
    /// more.
    pub async fn next(&mut self) -> Option<CommandOutputUpdate> {
        if !self.started {
            self.started = true;
            return Some(CommandOutputUpdate::Command {
                command: self.followed.view().await,
            });
        }
        loop {
            if let Some(update) = self.pending.pop_front() {
                return Some(update);
            }
            if self.done {
                return None;
            }
            // Lines that arrive after this mark change it again.
            let mark = *self.marks.borrow_and_update();
            let lines = self.log.lines_after(self.sent);
            self.queue(lines);
            if !self.pending.is_empty() {
                continue;
            }
            if mark.ended || self.marks.changed().await.is_err() {
                self.done = true;
                return Some(CommandOutputUpdate::Ended {
                    command: self.followed.ended().await,
                });
            }
        }
    }

    /// Queues lines in their order, with the lines that the log let go where
    /// they would be.
    fn queue(&mut self, lines: LogLines) {
        let gap = lines.left_out;
        let mut batch: Vec<String> = Vec::new();
        let mut first = 0;
        let mut gap_queued = gap.is_none();
        for (number, text) in lines.lines {
            if !gap_queued
                && let Some((gap_first, gap_last)) = gap
                && number > gap_last
            {
                self.flush(&mut batch, first);
                self.pending.push_back(CommandOutputUpdate::LeftOut {
                    first_line: line_number(gap_first),
                    last_line: line_number(gap_last),
                });
                gap_queued = true;
            }
            if batch.is_empty() {
                first = number;
            }
            batch.push(text);
            if batch.len() == UPDATE_LINES {
                self.flush(&mut batch, first);
            }
            self.sent = self.sent.max(number);
        }
        self.flush(&mut batch, first);
        if !gap_queued && let Some((gap_first, gap_last)) = gap {
            self.pending.push_back(CommandOutputUpdate::LeftOut {
                first_line: line_number(gap_first),
                last_line: line_number(gap_last),
            });
        }
        if let Some((_, gap_last)) = gap {
            self.sent = self.sent.max(gap_last);
        }
    }

    fn flush(&mut self, batch: &mut Vec<String>, first: u64) {
        if batch.is_empty() {
            return;
        }
        self.pending.push_back(CommandOutputUpdate::Lines {
            first_line: line_number(first),
            lines: std::mem::take(batch),
        });
    }
}

impl Runtime {
    /// Stops a background command at the owner's request, also one of a
    /// child's task conversation, and returns how it ended. A command that
    /// ended already is left as it is.
    pub async fn stop_background_command(
        &self,
        input: StopBackgroundCommand,
    ) -> Result<BackgroundCommandView, ToolsError> {
        let stopped = self
            .stop_background(
                &input.conversation_id,
                input.number,
                BackgroundEnding::StoppedByOwner,
            )
            .await;
        match stopped {
            Ok(stopped) => Ok(stopped.view),
            Err(ToolFailure::CommandNotFound { .. }) => Err(ToolsError::BackgroundCommandNotFound),
            Err(_failure) => Err(ToolsError::StorageFailed),
        }
    }

    /// Follows the output of a background command of this machine: from its
    /// log while it runs, from its archive once it ended.
    pub async fn command_output(
        &self,
        input: SubscribeCommandOutput,
    ) -> Result<CommandOutput, ToolsError> {
        let conversation = &input.conversation_id;
        let (followed, log) = match self.inner.background.tracked(conversation, input.number) {
            Some(tracked) => {
                let log = tracked.log.clone();
                (Followed::Running(tracked), log)
            }
            None => self.ended_log(conversation, input.number).await?,
        };
        let sent = match input.after_line {
            Some(line) => u64::from(line),
            None => log
                .mark()
                .lines
                .saturating_sub(u64::from(MAX_OUTPUT_VIEW_LINES)),
        };
        Ok(CommandOutput::new(followed, log, sent))
    }

    /// The view and the log of a command that ended, from its archive, or an
    /// empty log when Mannie's end lost its output.
    async fn ended_log(
        &self,
        conversation: &ConversationId,
        number: u32,
    ) -> Result<(Followed, LiveLog), ToolsError> {
        let store = &self.inner.store;
        let failed = |error: mannie_store::RuntimeStoreError| {
            tracing::error!(%error, error.code = error.code(), "could not read a background command's output");
            ToolsError::StorageFailed
        };
        let stored = store
            .background_command(conversation, number)
            .await
            .map_err(failed)?
            .ok_or(ToolsError::BackgroundCommandNotFound)?;
        let view = stored.view();
        let document = stored.document;
        let archive = match &document.archive_id {
            Some(archive_id) => store
                .archive(conversation, archive_id)
                .await
                .map_err(failed)?,
            None => None,
        };
        let log = if let Some(archive) = archive {
            LiveLog::from_archive(&archive.content, document.head_lines, document.left_out)
        } else {
            let lost = LiveLog::new();
            lost.end();
            lost
        };
        Ok((Followed::Ended(Box::new(view)), log))
    }
}

#[cfg(test)]
mod tests {
    use mannie_tools::LiveLog;
    use mannie_types::clock::UtcMillis;
    use mannie_types::ids::PartId;
    use mannie_types::tools::{
        BackgroundCommandView, BackgroundEnding, BackgroundSource, CommandOutputUpdate,
    };

    use super::{CommandOutput, Followed};

    fn ended_view() -> BackgroundCommandView {
        BackgroundCommandView {
            number: 1,
            part_id: PartId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap(),
            command: "make".to_owned(),
            source: BackgroundSource::Local {
                shell: "/bin/sh".to_owned(),
                working_directory: "/work".to_owned(),
            },
            started_at: UtcMillis::EPOCH,
            ending: Some(BackgroundEnding::Exited),
            exit_code: Some(0),
            signal: None,
            ended_at: Some(UtcMillis::EPOCH),
            lines: 3,
            bytes: 0,
            archive_id: None,
            on_worker: false,
        }
    }

    fn ended(log: LiveLog, sent: u64) -> CommandOutput {
        CommandOutput::new(Followed::Ended(Box::new(ended_view())), log, sent)
    }

    #[tokio::test]
    async fn the_output_of_an_ended_command_comes_in_order_then_its_end() {
        let log = LiveLog::from_archive(
            "one\ntwo\n[Lines 3 to 9 were left out]\nten\n",
            2,
            Some((3, 9)),
        );
        let mut output = ended(log, 0);

        let mut updates = Vec::new();
        while let Some(update) = output.next().await {
            updates.push(update);
        }

        assert_eq!(
            updates,
            [
                CommandOutputUpdate::Command {
                    command: ended_view(),
                },
                CommandOutputUpdate::Lines {
                    first_line: 1,
                    lines: vec!["one".to_owned(), "two".to_owned()],
                },
                CommandOutputUpdate::LeftOut {
                    first_line: 3,
                    last_line: 9,
                },
                CommandOutputUpdate::Lines {
                    first_line: 10,
                    lines: vec!["ten".to_owned()],
                },
                CommandOutputUpdate::Ended {
                    command: ended_view(),
                },
            ]
        );
    }

    #[tokio::test]
    async fn a_view_that_holds_lines_receives_only_the_later_ones() {
        let log = LiveLog::from_archive("one\ntwo\nthree\n", 3, None);
        let mut output = ended(log, 2);
        output.next().await;

        assert_eq!(
            output.next().await,
            Some(CommandOutputUpdate::Lines {
                first_line: 3,
                lines: vec!["three".to_owned()],
            })
        );
        assert_eq!(
            output.next().await,
            Some(CommandOutputUpdate::Ended {
                command: ended_view()
            })
        );
        assert_eq!(output.next().await, None);
    }

    #[tokio::test]
    async fn a_lost_output_ends_at_once() {
        let log = LiveLog::new();
        log.end();
        let mut output = ended(log, 0);
        output.next().await;

        assert_eq!(
            output.next().await,
            Some(CommandOutputUpdate::Ended {
                command: ended_view()
            })
        );
    }
}
