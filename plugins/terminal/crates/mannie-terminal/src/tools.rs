//! The tools `list_terminals` and `run_in_terminal`
//! (plugins/terminal/docs/README.md, agents' commands; ADR-0053, decision 5).
//!
//! `run_in_terminal`'s preparation reads only the plugin's terminals and the
//! core's hosts: it returns a terminal-command access that names the host,
//! the terminal, its shell, the user and folder that it reported last, and
//! the command, which the core's policy decides on. The execution acts only
//! on the access that the core authorized: it opens a new terminal when the
//! access names none, claims the terminal's prompt only when it is idle and
//! in the state that the access names, types the line and Enter, and waits
//! for the end marker. The timeout, and a Stop that drops the call, send
//! Ctrl+C.
//!
//! The result holds the output with numbered lines, its cite link, and the
//! output as a cited item, which the transcript's citations keep after the
//! terminal ended (ADR-0054, decision 1).

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::ArchiveId;
use mannie_plugin_sdk::points::context::ContextItem;
use mannie_plugin_sdk::points::tools::{
    CitedItem, TerminalCommandAccess, Tool, ToolAccess, ToolArchive, ToolCall, ToolExecution,
    ToolRefusal, ToolResult, ToolSpec,
};
use mannie_plugin_sdk::services::terminals::{TerminalHostState, TerminalSize};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::blocks::Author;
use crate::cite;
use crate::commands::TerminalPhase;
use crate::registry::{Entry, HostChoice, OpenRequest, SessionHost, Terminals};
use crate::schema::parameters;
use crate::session::{BlockOutcome, Refusal, Session};
use crate::texts;

/// The name of the tool that runs a command.
pub const RUN_IN_TERMINAL: &str = "run_in_terminal";

/// The name of the tool that lists the terminals.
pub const LIST_TERMINALS: &str = "list_terminals";

/// The most bytes of a command.
const MAX_COMMAND_BYTES: usize = 16 * 1024;

/// The wait of a call that names none.
const DEFAULT_TIMEOUT_SECONDS: u32 = 60;

/// The longest wait.
const MAX_TIMEOUT_SECONDS: u32 = 600;

/// How long a command gets to end after Ctrl+C.
const INTERRUPT_GRACE: Duration = Duration::from_secs(5);

/// How long a new terminal gets to reach a prompt that marks commands.
const PROMPT_WAIT: Duration = Duration::from_secs(15);

/// The size of a terminal that an agent opens, until a view resizes it.
const AGENT_SIZE: TerminalSize = TerminalSize {
    columns: 120,
    rows: 30,
};

/// The name of this computer in the arguments.
const THIS_COMPUTER: &str = "this computer";

/// The most characters of a command in a cite link's label.
const MAX_LABEL_CHARS: usize = 60;

/// The arguments of `run_in_terminal`. The field documentation is what the
/// model reads about each argument.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RunArguments {
    /// The command, as one line that the terminal's shell reads: at most
    /// 16 KiB, without line breaks.
    command: String,
    /// The number of a terminal of this conversation, as `list_terminals`
    /// gives it; null to use your own idle terminal on the host, or a new
    /// one.
    #[serde(default)]
    terminal: Option<u32>,
    /// Where the terminal runs when terminal is null: "this computer" or the
    /// name of a host from `list_terminals`; null for this computer.
    #[serde(default)]
    host: Option<String>,
    /// The shell of a new terminal on this computer, by its name from
    /// `list_terminals`; null for the default shell.
    #[serde(default)]
    shell: Option<String>,
    /// Seconds to wait for the command to end, from 1 to 600; 60 when null.
    /// A command that still runs then is interrupted with Ctrl+C.
    #[serde(default)]
    timeout_seconds: Option<u32>,
}

/// The arguments of `list_terminals`, which takes none.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "the braces make the arguments an object: a unit struct's schema is `null`, which providers refuse with the whole request, and it reads no `{}`"
)]
struct ListArguments {}

/// Where a call's command goes.
#[derive(Debug, Clone)]
enum Target {
    /// An existing terminal of the conversation.
    Existing(Arc<Entry>),
    /// A new terminal on a host.
    New { host: SessionHost, shell: String },
}

/// A call whose arguments passed the checks.
#[derive(Debug)]
struct Checked {
    command: String,
    target: Target,
    timeout_seconds: u32,
}

/// The tool that runs a command.
#[derive(Debug, Clone)]
pub(crate) struct RunInTerminal {
    terminals: Terminals,
}

impl RunInTerminal {
    pub(crate) fn new(terminals: Terminals) -> Self {
        Self { terminals }
    }
}

impl Tool for RunInTerminal {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            RUN_IN_TERMINAL,
            texts::RUN_DESCRIPTION,
            parameters::<RunArguments>(),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let conversation = call.conversation_id.to_string();
        let prepared = check(&self.terminals, &conversation, &call.arguments)
            .map(|checked| vec![ToolAccess::TerminalCommand(access(&checked))]);
        Box::pin(std::future::ready(prepared))
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let terminals = self.terminals.clone();
        Box::pin(async move { run(&terminals, execution).await })
    }
}

/// Reads and checks a call's arguments against the terminals as they are.
fn check(
    terminals: &Terminals,
    conversation: &str,
    arguments: &str,
) -> Result<Checked, ToolRefusal> {
    let arguments: RunArguments = serde_json::from_str(arguments)
        .map_err(|error| ToolRefusal::new(texts::invalid_arguments(&error.to_string())))?;
    check_command(&arguments.command)?;
    let timeout_seconds = arguments.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS);
    if !(1..=MAX_TIMEOUT_SECONDS).contains(&timeout_seconds) {
        return Err(ToolRefusal::new(texts::TIMEOUT_OUT_OF_RANGE));
    }
    let target = if let Some(number) = arguments.terminal {
        let entry = terminals
            .entry(conversation, number)
            .ok_or_else(|| ToolRefusal::new(texts::unknown_terminal(number)))?;
        let status = entry.session.status();
        if status.phase == TerminalPhase::Ended {
            return Err(ToolRefusal::new(texts::ended_terminal(number)));
        }
        if !status.marking {
            return Err(ToolRefusal::new(texts::plain_terminal(number)));
        }
        Target::Existing(entry)
    } else {
        let (host, shell) = host_of(
            terminals,
            arguments.host.as_deref(),
            arguments.shell.as_deref(),
        )?;
        match terminals.idle_agent_terminal(conversation, &host) {
            Some(entry)
                if arguments
                    .shell
                    .as_deref()
                    .is_none_or(|name| name == entry.shell()) =>
            {
                Target::Existing(entry)
            }
            _ => Target::New { host, shell },
        }
    };
    Ok(Checked {
        command: arguments.command,
        target,
        timeout_seconds,
    })
}

/// Checks a command's text: one line, not blank, within its size, without
/// control characters other than tabs.
fn check_command(command: &str) -> Result<(), ToolRefusal> {
    if command.trim().is_empty() {
        return Err(ToolRefusal::new(texts::BLANK_COMMAND));
    }
    if command.len() > MAX_COMMAND_BYTES {
        return Err(ToolRefusal::new(texts::LONG_COMMAND));
    }
    if command
        .chars()
        .any(|character| character.is_control() && character != '\t')
    {
        return Err(ToolRefusal::new(texts::COMMAND_NOT_ONE_LINE));
    }
    Ok(())
}

/// The host and shell that the arguments name for a new terminal.
fn host_of(
    terminals: &Terminals,
    host: Option<&str>,
    shell: Option<&str>,
) -> Result<(SessionHost, String), ToolRefusal> {
    let list = terminals.hosts();
    let on_this_computer = host.is_none_or(|name| name.eq_ignore_ascii_case(THIS_COMPUTER));
    if on_this_computer {
        let local = match shell {
            Some(name) => list
                .shells
                .iter()
                .find(|candidate| candidate.name.eq_ignore_ascii_case(name))
                .cloned()
                .ok_or_else(|| {
                    let names: Vec<String> =
                        list.shells.iter().map(|shell| shell.name.clone()).collect();
                    ToolRefusal::new(texts::unknown_shell(name, &names))
                })?,
            None => terminals
                .local_shell(None)
                .ok_or_else(|| ToolRefusal::new(texts::unknown_shell("default", &[])))?,
        };
        return Ok((
            SessionHost::ThisComputer {
                program: local.program,
            },
            local.name,
        ));
    }
    if shell.is_some() {
        return Err(ToolRefusal::new(texts::SHELL_ON_REMOTE));
    }
    let name = host.unwrap_or_default();
    let offered = list
        .hosts
        .iter()
        .find(|candidate| candidate.label.eq_ignore_ascii_case(name))
        .cloned()
        .ok_or_else(|| {
            let names: Vec<String> = list.hosts.iter().map(|host| host.label.clone()).collect();
            ToolRefusal::new(texts::unknown_host(name, &names))
        })?;
    if let TerminalHostState::Unavailable { code } = &offered.state {
        return Err(ToolRefusal::new(texts::unavailable_host(
            &offered.label,
            code,
        )));
    }
    let shell = offered
        .shell
        .map_or(texts::LOGIN_SHELL, crate::commands::shell_kind_name)
        .to_owned();
    Ok((
        SessionHost::Remote {
            plugin: offered.plugin,
            key: offered.key,
            label: offered.label,
            endpoint: offered.endpoint,
        },
        shell,
    ))
}

/// The command in its terminal, as the core's policy decides on it.
fn access(checked: &Checked) -> TerminalCommandAccess {
    match &checked.target {
        Target::Existing(entry) => {
            let access = TerminalCommandAccess::new(
                entry.host.host_ref(),
                Some(entry.number),
                entry.shell(),
                checked.command.clone(),
            );
            match entry.session.status().state {
                Some(state) => access.with_state(state),
                None => access,
            }
        }
        Target::New { host, shell } => TerminalCommandAccess::new(
            host.host_ref(),
            None,
            shell.clone(),
            checked.command.clone(),
        ),
    }
}

/// The authorized access of a call, when it asks for the same command as a
/// fresh preparation of it.
fn authorized<'a>(
    execution: &'a ToolExecution,
    prepared: &TerminalCommandAccess,
) -> Option<&'a TerminalCommandAccess> {
    execution.authorized.iter().find_map(|access| match access {
        ToolAccess::TerminalCommand(authorized) if authorized.same_request(prepared) => {
            Some(authorized)
        }
        _ => None,
    })
}

/// A result that says why the command did not run, with the plugin's code
/// for the card.
fn not_run(reason: &str, code: &str) -> ToolResult {
    let mut result = ToolResult::new(texts::not_run(reason));
    result.failure = Some(code.to_owned());
    result
}

/// Sends Ctrl+C when a call that typed its command is dropped before the
/// command ended, as Stop and Shutdown do (ADR-0034, decision 3).
struct InterruptOnDrop {
    session: Option<Arc<Session>>,
}

impl InterruptOnDrop {
    fn disarm(&mut self) {
        self.session = None;
    }
}

impl Drop for InterruptOnDrop {
    fn drop(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _interrupted = session.write(vec![0x03]).await;
            });
        }
    }
}

/// Runs an authorized call.
async fn run(terminals: &Terminals, execution: ToolExecution) -> ToolResult {
    let conversation = execution.conversation_id.to_string();
    let checked = match check(terminals, &conversation, &execution.arguments) {
        Ok(checked) => checked,
        Err(refusal) => return ToolResult::new(texts::not_run(&refusal.detail)),
    };
    let prepared = access(&checked);
    let Some(granted) = authorized(&execution, &prepared).cloned() else {
        return not_run(texts::CHANGED_SINCE_APPROVAL, "terminal_changed");
    };
    let entry = match &checked.target {
        Target::Existing(entry) => Arc::clone(entry),
        Target::New { host, .. } => {
            match open_new(terminals, &conversation, host, &granted, &execution).await {
                Ok(entry) => entry,
                Err(result) => return *result,
            }
        }
    };
    let number = entry.number;
    let expected = granted.state.clone();
    let command = checked.command.clone();
    let claim = entry
        .session
        .ask(move |core| core.claim(expected.as_deref(), command))
        .await;
    let outcome = match claim {
        Some(Ok(outcome)) => outcome,
        Some(Err(refusal)) => return refused(number, &refusal),
        None => return not_run(&texts::ended_terminal(number), "terminal_ended"),
    };
    let mut guard = InterruptOnDrop {
        session: Some(Arc::clone(&entry.session)),
    };
    tracing::info!(
        terminal = number,
        "an agent's command is typed into a terminal"
    );
    let mut line = checked.command.clone().into_bytes();
    line.push(b'\r');
    if entry.session.write(line).await.is_err() {
        guard.disarm();
        let releasing = entry.session.ask(|core| {
            core.release_claim();
        });
        let _released = releasing.await;
        return not_run(texts::NOT_TYPED, "terminal_ended");
    }
    let started = Instant::now();
    let (finished, timed_out) =
        wait_for_end(&entry.session, outcome, checked.timeout_seconds).await;
    guard.disarm();
    tracing::info!(
        terminal = number,
        elapsed_ms = started.elapsed().as_millis(),
        "an agent's command ended"
    );
    match finished {
        Some(finished) => result(
            &entry,
            &finished,
            timed_out.then_some(checked.timeout_seconds),
            &execution,
        ),
        None if timed_out => {
            let mut result = ToolResult::new(texts::still_running(number, checked.timeout_seconds));
            result.failure = Some("still_running".to_owned());
            result
        }
        None => not_run(&texts::ended_terminal(number), "terminal_ended"),
    }
}

/// Opens a new terminal for an agent's command on the host that the access
/// names, with the endpoint and identity that the core authorized, and waits
/// for its first prompt that marks commands. A failure is the call's result.
async fn open_new(
    terminals: &Terminals,
    conversation: &str,
    host: &SessionHost,
    granted: &TerminalCommandAccess,
    execution: &ToolExecution,
) -> Result<Arc<Entry>, Box<ToolResult>> {
    let choice = match host {
        SessionHost::ThisComputer { program } => HostChoice::ThisComputer {
            shell: Some(program.clone()),
        },
        SessionHost::Remote { plugin, key, .. } => HostChoice::Plugin {
            plugin: plugin.clone(),
            key: key.clone(),
        },
    };
    let request = OpenRequest {
        conversation: conversation.to_owned(),
        host: choice,
        folder: execution.workspace.clone(),
        size: AGENT_SIZE,
        opened_by: Author::Agent,
        expect: granted.expect.clone(),
    };
    let entry = terminals.open(request).await.map_err(|error| {
        Box::new(not_run(
            &texts::open_failed(error_code(&error)),
            "terminal_not_opened",
        ))
    })?;
    if !reached_prompt(&entry.session).await {
        return Err(Box::new(not_run(
            &texts::no_prompt(entry.number),
            "terminal_plain",
        )));
    }
    Ok(entry)
}

/// Waits for a typed command's end, at most `timeout_seconds`; then sends
/// Ctrl+C and waits a little more. Returns how it ended, if it did, and
/// whether the timeout passed.
async fn wait_for_end(
    session: &Session,
    mut outcome: tokio::sync::oneshot::Receiver<BlockOutcome>,
    timeout_seconds: u32,
) -> (Option<BlockOutcome>, bool) {
    let wait = Duration::from_secs(u64::from(timeout_seconds));
    match tokio::time::timeout(wait, &mut outcome).await {
        Ok(Ok(finished)) => (Some(finished), false),
        Ok(Err(_dropped)) => (None, false),
        Err(_elapsed) => {
            let _interrupted = session.write(vec![0x03]).await;
            match tokio::time::timeout(INTERRUPT_GRACE, &mut outcome).await {
                Ok(Ok(finished)) => (Some(finished), true),
                _ => (None, true),
            }
        }
    }
}

/// Waits until a new terminal's shell reaches a prompt that marks commands,
/// and says whether it did.
async fn reached_prompt(session: &Session) -> bool {
    let mut status = session.follow_status();
    let waiting = async {
        loop {
            {
                let current = status.borrow_and_update();
                if current.phase == TerminalPhase::Ended {
                    return false;
                }
                if current.marking && current.phase == TerminalPhase::Prompt {
                    return true;
                }
            }
            if status.changed().await.is_err() {
                return false;
            }
        }
    };
    tokio::time::timeout(PROMPT_WAIT, waiting)
        .await
        .unwrap_or(false)
}

/// The result of a claim that the terminal refused.
fn refused(number: u32, refusal: &Refusal) -> ToolResult {
    match refusal {
        Refusal::Ended => not_run(&texts::ended_terminal(number), "terminal_ended"),
        Refusal::NotMarking => not_run(&texts::plain_terminal(number), "terminal_plain"),
        Refusal::Busy => not_run(&texts::busy_terminal(number), "terminal_busy"),
        Refusal::OwnerTyping => not_run(&texts::owner_typing(number), "owner_typing"),
        Refusal::Changed { now } => not_run(
            &texts::state_changed(number, now.as_deref()),
            "terminal_changed",
        ),
    }
}

/// The code of an error that kept a terminal from opening.
fn error_code(error: &crate::commands::TerminalError) -> &'static str {
    use mannie_plugin_sdk::ipc::PluginError as _;
    error.code().trim_start_matches("terminal.")
}

/// Where a terminal runs, as the result names it.
fn place(entry: &Entry) -> String {
    format!("{} on {}", entry.shell(), entry.host.label())
}

/// The result of a command that ended: its first line, its cite link, the
/// output with numbered lines within the result's room, the rest in an
/// archive, and the output as a cited item.
fn result(
    entry: &Entry,
    finished: &BlockOutcome,
    interrupted_after: Option<u32>,
    execution: &ToolExecution,
) -> ToolResult {
    let block = &finished.block;
    let duration = texts::duration(block.duration_ms().unwrap_or(0));
    let ending = match (interrupted_after, block.exit_code) {
        (Some(timeout), _) => texts::interrupted(timeout),
        (None, Some(code)) => texts::exited(code, &duration),
        (None, None) => texts::finished(&duration),
    };
    let key = cite::block_key(&entry.session.id, block.number);
    let label: String = block.command.chars().take(MAX_LABEL_CHARS).collect();
    let link = cite::block_link(&key, &label);
    let mut output = texts::ran(
        &block.command,
        entry.number,
        &place(entry),
        block.folder.as_deref(),
        &ending,
    );
    output.push('\n');
    output.push_str(&texts::cite_hint(&link, &key));
    output.push_str("\n\n");
    let lines = &finished.text.lines;
    let mut archives = Vec::new();
    if lines.is_empty() {
        output.push_str(texts::NO_OUTPUT);
    } else {
        output.push_str(&texts::output_heading(lines.len(), finished.text.omitted));
        output.push('\n');
        let numbered = numbered(lines);
        let room = (execution.result_limit as usize).saturating_sub(output.len() + 400);
        if numbered.len() <= room {
            output.push_str(&numbered);
        } else {
            let archive = execution.archive_ids.first().cloned();
            output.push_str(&fitted(&numbered, room));
            if let Some(archive) = archive {
                output.push('\n');
                output.push_str(&texts::output_archived(archive.as_str()));
                archives.push(archived(archive, numbered));
            }
        }
    }
    let item = ContextItem::new(key.clone(), label, None, finished.text.joined());
    let mut result = ToolResult::new(output).with_cited(vec![CitedItem::new(cite::PROVIDER, item)]);
    result.archives = archives;
    result.data = Some(json!({
        "terminal": entry.number,
        "block": block.number,
        "command": block.command,
        "exitCode": block.exit_code,
        "durationMs": block.duration_ms(),
        "interrupted": interrupted_after.is_some(),
        "host": entry.host.label(),
        "shell": entry.shell(),
        "session": entry.session.id,
        "key": key,
    }));
    if interrupted_after.is_some() {
        result.failure = Some("interrupted".to_owned());
    }
    result
}

/// The lines with their numbers, as `read_file` numbers them.
fn numbered(lines: &[String]) -> String {
    let mut text = String::new();
    for (index, line) in lines.iter().enumerate() {
        let _written = writeln!(text, "{:>6}\t{line}", index + 1);
    }
    text
}

/// The start and end of a text within `room` bytes, cut at line breaks.
fn fitted(text: &str, room: usize) -> String {
    let head_room = room * 2 / 5;
    let tail_room = room - head_room;
    let head_end = text[..head_room.min(text.len())]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let tail_from = text.len().saturating_sub(tail_room);
    let tail_start = text[tail_from..]
        .find('\n')
        .map_or(text.len(), |index| tail_from + index + 1);
    format!(
        "{}…\n{}",
        &text[..head_end],
        &text[tail_start.max(head_end)..]
    )
}

/// The archive of an output, within the archive's size.
fn archived(id: ArchiveId, mut text: String) -> ToolArchive {
    let limit = mannie_plugin_sdk::points::tools::MAX_TOOL_ARCHIVE_BYTES;
    if text.len() > limit {
        let mut end = limit;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    ToolArchive::new(id, "output", text)
}

/// The tool that lists the terminals.
#[derive(Debug, Clone)]
pub(crate) struct ListTerminals {
    terminals: Terminals,
}

impl ListTerminals {
    pub(crate) fn new(terminals: Terminals) -> Self {
        Self { terminals }
    }
}

impl Tool for ListTerminals {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            LIST_TERMINALS,
            texts::LIST_DESCRIPTION,
            parameters::<ListArguments>(),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let arguments = call.arguments.trim();
        let checked = if arguments.is_empty() || arguments == "null" {
            Ok(Vec::new())
        } else {
            serde_json::from_str::<ListArguments>(arguments)
                .map(|_arguments| Vec::new())
                .map_err(|error| ToolRefusal::new(texts::invalid_arguments(&error.to_string())))
        };
        Box::pin(std::future::ready(checked))
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let text = list(&self.terminals, &execution.conversation_id.to_string());
        Box::pin(std::future::ready(ToolResult::new(text)))
    }
}

/// The conversation's terminals and the hosts, as the model reads them.
fn list(terminals: &Terminals, conversation: &str) -> String {
    let mut text = String::new();
    let entries = terminals.of(conversation);
    if entries.is_empty() {
        text.push_str("This conversation has no terminals.\n");
    } else {
        text.push_str("This conversation's terminals:\n");
        for entry in entries {
            let status = entry.session.status();
            let phase = match status.phase {
                TerminalPhase::Starting => "starting",
                TerminalPhase::Prompt => "at its prompt",
                TerminalPhase::Running => "running a command",
                TerminalPhase::Ended => "ended",
            };
            let by = match entry.opened_by {
                Author::Owner => "the owner",
                Author::Agent => "you",
            };
            let _written = write!(
                text,
                "- Terminal {}: {}, opened by {by}, {phase}",
                entry.number,
                place(&entry)
            );
            if let Some(state) = &status.state {
                let _written = write!(text, ", as {state}");
            }
            if status.phase != TerminalPhase::Ended && !status.marking {
                text.push_str("; its shell does not mark commands, so commands cannot run there");
            }
            text.push_str(".\n");
        }
    }
    let hosts = terminals.hosts();
    let shells: Vec<String> = hosts
        .shells
        .iter()
        .map(|shell| shell.name.clone())
        .collect();
    let remote: Vec<String> = hosts
        .hosts
        .iter()
        .filter(|host| matches!(host.state, TerminalHostState::Ready))
        .map(|host| host.label.clone())
        .collect();
    text.push_str(&texts::section_hosts(&shells, &remote));
    text
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ListArguments, RunArguments, check_command, fitted, numbered};
    use crate::schema::parameters;

    #[test]
    fn both_tools_take_an_object_of_arguments_as_providers_require() {
        let list = parameters::<ListArguments>();
        let run = parameters::<RunArguments>();

        assert_eq!(
            list,
            json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false,
            })
        );
        serde_json::from_str::<ListArguments>("{}").unwrap();
        serde_json::from_str::<ListArguments>(r#"{"host": "x"}"#).unwrap_err();
        assert_eq!(run["type"], json!("object"));
        assert_eq!(
            run["required"],
            json!(["command", "host", "shell", "terminal", "timeout_seconds"])
        );
        assert_eq!(run["additionalProperties"], json!(false));
    }

    #[test]
    fn a_command_is_one_line_within_its_size() {
        check_command("Get-NetTCPConnection -State Listen | Sort-Object LocalPort").unwrap();
        check_command("printf 'a\tb'").unwrap();
        assert!(check_command("  ").is_err());
        assert!(check_command("ls\nrm -rf /").is_err());
        assert!(check_command("ls\u{1b}[2J").is_err());
        assert!(check_command(&"x".repeat(16 * 1024 + 1)).is_err());
    }

    #[test]
    fn lines_are_numbered_and_a_long_output_keeps_its_ends() {
        let lines: Vec<String> = (1..=1000).map(|line| format!("line {line}")).collect();
        let text = numbered(&lines);
        assert!(text.starts_with("     1\tline 1\n"));

        let cut = fitted(&text, 2000);

        assert!(cut.len() <= 2100);
        assert!(cut.starts_with("     1\tline 1\n"));
        assert!(cut.trim_end().ends_with("line 1000"));
        assert!(cut.contains("…\n"));
    }
}
