//! The mention provider `terminal` (ADR-0032; plugins/terminal/docs/
//! README.md, pointing an agent at output): "@" in the message box lists the
//! conversation's recent commands, and the terminal's panel mentions a
//! command or a selection through the message box (ADR-0054, decision 4).
//! A mention gives the model the terminal, its host, the command, its exit
//! code and its output, or the selection, with the command's cite link.
//!
//! This is the only way that output the agent did not produce reaches the
//! model: the owner chooses each mention (D45).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{MentionEntry, MentionSummary};
use mannie_plugin_sdk::points::mentions::{
    MentionContent, MentionContext, MentionProvider, ProviderSuggestions, RenderedMention,
};

use crate::blocks::{Author, Block, BlockText};
use crate::cite::{self, Key};
use crate::registry::{Entry, Terminals};

/// The provider's ID, which is the plugin's.
pub(crate) const PROVIDER_ID: &str = "terminal";

/// The variant of a command's entries, which chooses its icon.
pub(crate) const COMMAND_VARIANT: &str = "command";

/// The variant of a selection's entries.
pub(crate) const SELECTION_VARIANT: &str = "selection";

/// The summary code of a mention, which the UI words with its `lines`.
pub(crate) const LINES_SUMMARY: &str = "lines";

/// The code of a key that names nothing the conversation's terminals hold.
pub(crate) const NOT_FOUND: &str = "not_found";

/// The most commands that "@" looks through, the latest first.
const MAX_LISTED: usize = 50;

/// The conversation's commands, as "@" lists them.
#[derive(Debug)]
pub(crate) struct CommandMentions {
    terminals: Terminals,
}

impl CommandMentions {
    pub(crate) fn new(terminals: Terminals) -> Self {
        Self { terminals }
    }
}

/// A command's mention entry.
pub(crate) fn block_entry(entry: &Entry, block: &Block) -> MentionEntry {
    let label: String = format!("Terminal {}: {}", entry.number, block.command)
        .chars()
        .take(200)
        .collect();
    MentionEntry {
        key: cite::block_key(&entry.session.id, block.number),
        label,
        token: format!("@terminal-{}:{}", entry.number, block.number),
        variant: COMMAND_VARIANT.to_owned(),
        warning: None,
        expands: false,
    }
}

/// A selection's mention entry.
pub(crate) fn selection_entry(entry: &Entry, key: String, text: &str) -> MentionEntry {
    let first_line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    let label: String = format!("Terminal {}: {}", entry.number, first_line.trim())
        .chars()
        .take(200)
        .collect();
    let suffix = key.rsplit('.').next().unwrap_or_default().to_owned();
    MentionEntry {
        key,
        label,
        token: format!("@terminal-{}:{suffix}", entry.number),
        variant: SELECTION_VARIANT.to_owned(),
        warning: None,
        expands: false,
    }
}

impl MentionProvider for CommandMentions {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn offered(&self, context: &MentionContext) -> bool {
        context.conversation().is_some()
    }

    fn suggest<'a>(
        &'a self,
        context: &'a MentionContext,
        query: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, Result<ProviderSuggestions, String>> {
        Box::pin(async move {
            let Some(conversation) = context.conversation() else {
                return Ok(ProviderSuggestions::default());
            };
            let query = query.to_lowercase();
            let mut found: Vec<(u64, MentionEntry)> = Vec::new();
            for entry in self.terminals.of(&conversation.to_string()) {
                let blocks = entry
                    .session
                    .ask(|core| core.blocks())
                    .await
                    .unwrap_or_default();
                for block in blocks.iter().rev().take(MAX_LISTED) {
                    let matches = query.is_empty() || block.command.to_lowercase().contains(&query);
                    if matches && !block.command.is_empty() {
                        let at = u64::try_from(block.started_at.as_millis()).unwrap_or(0);
                        found.push((at, block_entry(&entry, block)));
                    }
                }
            }
            found.sort_by_key(|(at, _entry)| std::cmp::Reverse(*at));
            let incomplete = found.len() > limit;
            let entries = found
                .into_iter()
                .take(limit)
                .map(|(_at, entry)| entry)
                .collect();
            Ok(ProviderSuggestions::new(entries, incomplete))
        })
    }

    fn resolve<'a>(
        &'a self,
        context: &'a MentionContext,
        key: &'a str,
        _max_bytes: usize,
    ) -> BoxFuture<'a, Result<Box<dyn MentionContent>, String>> {
        Box::pin(async move {
            let conversation = context.conversation().ok_or_else(|| NOT_FOUND.to_owned())?;
            let parsed = cite::parse(key).ok_or_else(|| NOT_FOUND.to_owned())?;
            let terminal = match parsed {
                Key::Block { terminal, .. } | Key::Selection { terminal } => terminal,
            };
            let entry = self
                .terminals
                .by_id(&conversation.to_string(), terminal)
                .ok_or_else(|| NOT_FOUND.to_owned())?;
            let mentioned: Box<dyn MentionContent> = match parsed {
                Key::Block { number, .. } => {
                    let (block, text) = entry
                        .session
                        .ask(move |core| core.block_text(number))
                        .await
                        .flatten()
                        .ok_or_else(|| NOT_FOUND.to_owned())?;
                    Box::new(MentionedBlock::of(&entry, &block, text))
                }
                Key::Selection { .. } => {
                    let wanted = key.to_owned();
                    let text = entry
                        .session
                        .ask(move |core| core.selection(&wanted))
                        .await
                        .flatten()
                        .ok_or_else(|| NOT_FOUND.to_owned())?;
                    Box::new(MentionedSelection::of(&entry, &text))
                }
            };
            Ok(mentioned)
        })
    }
}

/// Where a terminal runs, as a mention tells it.
fn place(entry: &Entry) -> String {
    format!(
        "terminal {} ({} on {})",
        entry.number,
        entry.shell(),
        entry.host.label()
    )
}

/// A mentioned command, read when the message was sent.
struct MentionedBlock {
    header: String,
    lines: Vec<String>,
    label: String,
}

impl MentionedBlock {
    fn of(entry: &Arc<Entry>, block: &Block, text: BlockText) -> Self {
        let key = cite::block_key(&entry.session.id, block.number);
        let label: String = block.command.chars().take(60).collect();
        let by = match block.author {
            Author::Owner => "the owner",
            Author::Agent => "you",
        };
        let mut header = format!(
            "The owner mentions a command that {by} ran in {}: `{}`",
            place(entry),
            block.command
        );
        match (block.end, block.exit_code) {
            (None, _) => header.push_str(", which still runs."),
            (Some(_), Some(code)) => {
                let _written = write!(header, ", which exited with {code}.");
            }
            (Some(_), None) => header.push_str(", which ended."),
        }
        if let Some(folder) = &block.folder {
            let _written = write!(header, " It ran in {folder}.");
        }
        let _written = write!(
            header,
            " Cite its output as {}, adding #L<first>-L<last> for lines.",
            cite::block_link(&key, &label)
        );
        Self {
            header,
            lines: text.lines,
            label: format!("Terminal {}: {}", entry.number, block.command),
        }
    }
}

impl MentionContent for MentionedBlock {
    fn render(&self, limit: usize) -> RenderedMention {
        let (text, shown) = render_lines(&self.header, &self.lines, limit);
        RenderedMention::new(
            text,
            self.label.clone(),
            COMMAND_VARIANT.to_owned(),
            summary(self.lines.len(), shown),
        )
    }
}

/// A mentioned selection.
struct MentionedSelection {
    header: String,
    lines: Vec<String>,
    label: String,
}

impl MentionedSelection {
    fn of(entry: &Arc<Entry>, text: &str) -> Self {
        Self {
            header: format!("The owner selected this text in {}:", place(entry)),
            lines: text.lines().map(str::to_owned).collect(),
            label: format!("Terminal {}: selection", entry.number),
        }
    }
}

impl MentionContent for MentionedSelection {
    fn render(&self, limit: usize) -> RenderedMention {
        let (text, shown) = render_lines(&self.header, &self.lines, limit);
        RenderedMention::new(
            text,
            self.label.clone(),
            SELECTION_VARIANT.to_owned(),
            summary(self.lines.len(), shown),
        )
    }
}

/// The header and as many numbered lines as fit `limit` bytes, and how many
/// lines it shows.
fn render_lines(header: &str, lines: &[String], limit: usize) -> (String, usize) {
    let mut text = String::from(header);
    text.push('\n');
    let mut shown = 0;
    for (index, line) in lines.iter().enumerate() {
        let numbered = format!("{:>6}\t{line}\n", index + 1);
        if text.len() + numbered.len() > limit {
            break;
        }
        text.push_str(&numbered);
        shown += 1;
    }
    if shown < lines.len() {
        let note = format!("({} of {} lines are shown.)", shown, lines.len());
        if text.len() + note.len() <= limit {
            text.push_str(&note);
        }
    }
    if text.len() > limit {
        let mut end = limit;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    (text, shown)
}

/// What the model got, which the UI words.
fn summary(total: usize, shown: usize) -> MentionSummary {
    MentionSummary {
        code: LINES_SUMMARY.to_owned(),
        values: BTreeMap::from([
            ("total".to_owned(), u32::try_from(total).unwrap_or(u32::MAX)),
            ("shown".to_owned(), u32::try_from(shown).unwrap_or(u32::MAX)),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::render_lines;

    #[test]
    fn a_mention_shows_the_lines_that_fit() {
        let lines: Vec<String> = (1..=100).map(|line| format!("line {line}")).collect();

        let (all, shown_all) = render_lines("Header", &lines, 100_000);
        let (some, shown_some) = render_lines("Header", &lines, 200);

        assert_eq!(shown_all, 100);
        assert!(all.contains("   100\tline 100"));
        assert!(shown_some < 100);
        assert!(some.len() <= 200);
        assert!(some.starts_with("Header\n     1\tline 1\n"));
    }
}
