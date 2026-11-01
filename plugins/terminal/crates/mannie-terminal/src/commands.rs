//! The plugin's commands and channels, as specs of the plugin SDK (ADR-0034,
//! decision 8), so the host checks their input before the plugin's handlers
//! run and answers `plugin_off` while the plugin is off.
//!
//! What the owner types reaches the shell as typed, without the policy
//! (ADR-0053, decision 6). The commands check that the terminal belongs to
//! the conversation and cap each input; nothing of the input is logged or
//! stored.

use mannie_plugin_sdk::contracts::{ConversationId, MentionEntry, UtcMillis};
use mannie_plugin_sdk::ipc::{
    ChannelSpec, CommandSpec, InputRefusal, Interface, PluginError, Validate,
};
use mannie_plugin_sdk::services::terminals::{
    MAX_TERMINAL_COLUMNS, MAX_TERMINAL_ROWS, MAX_TERMINAL_WRITE_BYTES, ShellKind,
};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::blocks::Author;

/// The fewest lines of scrollback that a view keeps.
pub const MIN_SCROLLBACK: u32 = 500;

/// The most lines of scrollback that a view keeps.
pub const MAX_SCROLLBACK: u32 = 20_000;

/// The lines of scrollback that a view keeps unless the settings say
/// otherwise.
pub const DEFAULT_SCROLLBACK: u32 = 2_000;

/// The smallest font size of a terminal, in pixels.
pub const MIN_FONT_SIZE: u32 = 10;

/// The largest font size of a terminal, in pixels.
pub const MAX_FONT_SIZE: u32 = 24;

/// The font size of a terminal unless the settings say otherwise.
pub const DEFAULT_FONT_SIZE: u32 = 13;

/// The most bytes of a selection that the owner mentions.
pub const MAX_SELECTION_BYTES: u32 = 64 * 1024;

/// The most bytes of one write to a terminal, as the UI checks it.
pub const MAX_WRITE_BYTES: u32 = 64 * 1024;

// The core's limit and the UI's are the same.
const _: () = assert!(MAX_WRITE_BYTES as usize == MAX_TERMINAL_WRITE_BYTES);

/// The most characters of a view's ID.
const MAX_VIEW_ID_CHARS: usize = 64;

/// The most characters of a path or a key that names a host or a shell.
const MAX_NAME_CHARS: usize = 4096;

/// A count that can pass 2^32 within a session, such as an absolute row or
/// the bytes sent to a view: a JSON number, which JavaScript reads exactly up
/// to 2^53.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Count(pub u64);

impl Type for Count {
    fn definition(types: &mut specta::Types) -> specta::datatype::DataType {
        // specta refuses 64-bit integers, because JavaScript numbers lose
        // precision above 2^53. A row or a byte count of one session stays
        // far below that, so the bindings type it as a plain `number`, like
        // the `u32` whose definition this borrows.
        <u32 as Type>::definition(types)
    }
}

/// Whether a size fits a terminal.
fn size_fits(columns: u16, rows: u16) -> bool {
    (1..=MAX_TERMINAL_COLUMNS).contains(&columns) && (1..=MAX_TERMINAL_ROWS).contains(&rows)
}

/// Whether a text names something: not blank, within its length, without
/// control characters.
fn is_name(text: &str) -> bool {
    let fits = !text.trim().is_empty() && text.chars().count() <= MAX_NAME_CHARS;
    fits && !text.chars().any(char::is_control)
}

/// Where the owner opens a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum HostChoice {
    /// This computer, in one of its shells, or the default shell.
    ThisComputer {
        /// The shell's program, as the list of hosts gives it.
        shell: Option<String>,
    },
    /// A host that a plugin offers.
    Plugin {
        /// The plugin.
        plugin: String,
        /// The plugin's key for the host.
        key: String,
    },
}

/// The argument of `open_terminal`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenTerminal {
    /// The conversation that the terminal belongs to.
    pub conversation_id: ConversationId,
    /// Where it opens.
    pub host: HostChoice,
    /// The folder that a terminal on this computer starts in, such as the
    /// project's folder, or `None` for the home folder.
    pub folder: Option<String>,
    /// The view's columns.
    pub columns: u16,
    /// The view's rows.
    pub rows: u16,
}

impl Validate for OpenTerminal {
    fn validate(&self) -> Result<(), InputRefusal> {
        let host_named = match &self.host {
            HostChoice::ThisComputer { shell } => shell.as_deref().is_none_or(is_name),
            HostChoice::Plugin { plugin, key } => is_name(plugin) && is_name(key),
        };
        if !host_named {
            return Err(InputRefusal::new("host"));
        }
        if self
            .folder
            .as_deref()
            .is_some_and(|folder| !is_name(folder))
        {
            return Err(InputRefusal::new("folder"));
        }
        if !size_fits(self.columns, self.rows) {
            return Err(InputRefusal::new("columns"));
        }
        Ok(())
    }
}

/// A terminal of a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalRef {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The terminal's number in it.
    pub terminal: u32,
}

impl Validate for TerminalRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.terminal == 0 {
            return Err(InputRefusal::new("terminal"));
        }
        Ok(())
    }
}

/// The argument of `write_terminal`: what the owner typed or pasted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteTerminal {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The terminal's number in it.
    pub terminal: u32,
    /// The text, as xterm.js encodes the keys: at most 64 KiB.
    pub data: String,
}

impl Validate for WriteTerminal {
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.terminal == 0 {
            return Err(InputRefusal::new("terminal"));
        }
        if self.data.is_empty() || self.data.len() > MAX_TERMINAL_WRITE_BYTES {
            return Err(InputRefusal::new("data"));
        }
        Ok(())
    }
}

/// The argument of `resize_terminal`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResizeTerminal {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The terminal's number in it.
    pub terminal: u32,
    /// The view's columns.
    pub columns: u16,
    /// The view's rows.
    pub rows: u16,
}

impl Validate for ResizeTerminal {
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.terminal == 0 {
            return Err(InputRefusal::new("terminal"));
        }
        if !size_fits(self.columns, self.rows) {
            return Err(InputRefusal::new("columns"));
        }
        Ok(())
    }
}

/// Whether a view's ID is one that the view made: letters, digits and
/// hyphens.
fn is_view_id(view: &str) -> bool {
    let fits = !view.is_empty() && view.len() <= MAX_VIEW_ID_CHARS;
    fits && view
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

/// The argument of `follow_terminal`: a view that shows a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FollowTerminal {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The terminal's number in it.
    pub terminal: u32,
    /// The view's own ID, which its acknowledgments name.
    pub view: String,
    /// The lines of scrollback that the view keeps, which its snapshots
    /// fill.
    pub scrollback: u32,
}

impl Validate for FollowTerminal {
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.terminal == 0 {
            return Err(InputRefusal::new("terminal"));
        }
        if !is_view_id(&self.view) {
            return Err(InputRefusal::new("view"));
        }
        if !(MIN_SCROLLBACK..=MAX_SCROLLBACK).contains(&self.scrollback) {
            return Err(InputRefusal::new("scrollback"));
        }
        Ok(())
    }
}

/// The argument of `acknowledge_output`: what a view has written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcknowledgeOutput {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The terminal's number in it.
    pub terminal: u32,
    /// The view.
    pub view: String,
    /// The `seq` of the last update that the view wrote.
    pub seq: Count,
}

impl Validate for AcknowledgeOutput {
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.terminal == 0 {
            return Err(InputRefusal::new("terminal"));
        }
        if !is_view_id(&self.view) {
            return Err(InputRefusal::new("view"));
        }
        Ok(())
    }
}

/// The argument of `follow_terminals`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationRef {
    /// The conversation.
    pub conversation_id: ConversationId,
}

impl Validate for ConversationRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// A command of a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BlockRef {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The terminal's number in it.
    pub terminal: u32,
    /// The command's number in the terminal.
    pub block: u32,
}

impl Validate for BlockRef {
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.terminal == 0 {
            return Err(InputRefusal::new("terminal"));
        }
        if self.block == 0 {
            return Err(InputRefusal::new("block"));
        }
        Ok(())
    }
}

/// The argument of `mention_selection`: text that the owner selected in a
/// terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MentionSelection {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The terminal's number in it.
    pub terminal: u32,
    /// The selected text: at most 64 KiB.
    pub text: String,
}

impl Validate for MentionSelection {
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.terminal == 0 {
            return Err(InputRefusal::new("terminal"));
        }
        let fits = self.text.len() <= MAX_SELECTION_BYTES as usize;
        if self.text.trim().is_empty() || !fits {
            return Err(InputRefusal::new("text"));
        }
        Ok(())
    }
}

/// How a terminal draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Renderer {
    /// The GPU's renderer where the webview has hardware acceleration, the
    /// compatible one otherwise.
    #[default]
    Automatic,
    /// Always the GPU's renderer.
    Gpu,
    /// Always the compatible renderer.
    Compatible,
}

/// The plugin's settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalSettings {
    /// The shell of this computer that a terminal runs when none is chosen,
    /// by its program, or `None` for the system's default.
    pub default_shell: Option<String>,
    /// The lines of scrollback that a view keeps.
    pub scrollback: u32,
    /// The font size, in pixels.
    pub font_size: u32,
    /// Whether the cursor blinks.
    pub cursor_blink: bool,
    /// How terminals draw.
    pub renderer: Renderer,
    /// Whether terminals serve screen readers, which costs some speed.
    pub screen_reader: bool,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            default_shell: None,
            scrollback: DEFAULT_SCROLLBACK,
            font_size: DEFAULT_FONT_SIZE,
            cursor_blink: false,
            renderer: Renderer::Automatic,
            screen_reader: false,
        }
    }
}

impl Validate for TerminalSettings {
    fn validate(&self) -> Result<(), InputRefusal> {
        if self
            .default_shell
            .as_deref()
            .is_some_and(|shell| !is_name(shell))
        {
            return Err(InputRefusal::new("defaultShell"));
        }
        if !(MIN_SCROLLBACK..=MAX_SCROLLBACK).contains(&self.scrollback) {
            return Err(InputRefusal::new("scrollback"));
        }
        if !(MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&self.font_size) {
            return Err(InputRefusal::new("fontSize"));
        }
        Ok(())
    }
}

/// A shell of this computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ShellView {
    /// The shell's program, which names it.
    pub program: String,
    /// The shell's name, such as `Windows PowerShell`.
    pub name: String,
    /// Whether a terminal runs it when none is chosen.
    pub default: bool,
}

/// Whether terminals open on a host now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum HostStateView {
    /// They open.
    Ready,
    /// They do not, for the host's plugin's code, such as `key_changed`.
    Unavailable {
        /// The plugin's code.
        code: String,
    },
}

/// A host that a plugin offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OfferedHostView {
    /// The plugin that offers it.
    pub plugin: String,
    /// The plugin's key for it.
    pub key: String,
    /// Its name, as the owner gave it.
    pub label: String,
    /// Where terminals run there.
    pub endpoint: String,
    /// Whether terminals open there now.
    pub state: HostStateView,
}

/// Where terminals open: this computer's shells and the hosts that plugins
/// offer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HostsView {
    /// This computer's shells, the default first.
    pub shells: Vec<ShellView>,
    /// The hosts that plugins offer.
    pub hosts: Vec<OfferedHostView>,
}

/// Where a terminal runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum TerminalHostView {
    /// This computer, in a shell.
    ThisComputer {
        /// The shell's program.
        program: String,
    },
    /// A host that a plugin offers.
    Remote {
        /// The plugin.
        plugin: String,
        /// The plugin's key for the host.
        key: String,
        /// The host's name.
        label: String,
        /// Where the terminal runs.
        endpoint: String,
    },
}

/// What a terminal does now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TerminalPhase {
    /// The shell starts.
    Starting,
    /// The shell waits at its prompt.
    Prompt,
    /// A command runs.
    Running,
    /// The terminal ended.
    Ended,
}

/// How a terminal ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EndingView {
    /// The shell exited with this code.
    Exited {
        /// The exit code.
        code: i32,
    },
    /// The terminal was ended.
    Closed,
    /// The plugin went off or Mannie quit.
    Stopped,
    /// The connection to its host was lost.
    Lost {
        /// The host's plugin's code.
        code: String,
    },
}

/// A terminal of a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSummary {
    /// Its number in the conversation.
    pub number: u32,
    /// Its own ID, which its cite links and mentions name.
    pub session: String,
    /// Where it runs.
    pub host: TerminalHostView,
    /// Its shell's name, or empty while a host's login shell is not known.
    pub shell: String,
    /// Who opened it.
    pub opened_by: Author,
    /// What it does now.
    pub phase: TerminalPhase,
    /// Whether its shell marks its commands, which agents need.
    pub marking: bool,
    /// The user and folder that it reported last, such as `root in /etc`.
    pub state: Option<String>,
    /// How it ended.
    pub ending: Option<EndingView>,
}

/// The update of `follow_terminals`: the conversation's terminals, each
/// item the complete list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TerminalListUpdate {
    /// The terminals, by number.
    pub terminals: Vec<TerminalSummary>,
}

/// A command of a terminal, as its view shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BlockView {
    /// Its number in the terminal.
    pub number: u32,
    /// Who ran it.
    pub author: Author,
    /// The command line.
    pub command: String,
    /// The exit code, once it ended and the shell gave one.
    pub exit_code: Option<i32>,
    /// Whether it still runs.
    pub running: bool,
    /// When it started.
    pub started_at: UtcMillis,
    /// How long it ran, in milliseconds, once it ended.
    pub duration_ms: Option<u32>,
    /// The absolute row where its output starts.
    pub first_row: Count,
    /// The absolute row after its output.
    pub end_row: Count,
    /// The key that its cite link and its mention name.
    pub key: String,
}

/// What a terminal's view receives. The view writes `Snapshot`, `Screen` and
/// `Output` data into xterm.js in order and acknowledges their `seq`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum TerminalUpdate {
    /// Rebuild the view: reset it, then write `data`, the latest lines of
    /// the terminal and its screen.
    Snapshot {
        /// The bytes sent to the view so far, this update included.
        seq: Count,
        /// What to write.
        data: String,
        /// The absolute row of the first line written.
        first_row: Count,
        /// The absolute row of the screen's first row.
        top_row: Count,
    },
    /// Write `data` over the screen; the view fell behind a flood, and its
    /// scrollback misses what scrolled meanwhile.
    Screen {
        /// The bytes sent to the view so far, this update included.
        seq: Count,
        /// What to write.
        data: String,
        /// The absolute row of the screen's first row.
        top_row: Count,
    },
    /// Write the terminal's output.
    Output {
        /// The bytes sent to the view so far, this update included.
        seq: Count,
        /// What to write.
        data: String,
        /// The absolute row of the screen's first row after it.
        top_row: Count,
    },
    /// The terminal's commands, the latest last.
    Blocks {
        /// The commands.
        blocks: Vec<BlockView>,
    },
    /// A command started or changed.
    Block {
        /// The command.
        block: BlockView,
    },
    /// What the terminal does changed.
    Status {
        /// What it does.
        phase: TerminalPhase,
        /// Whether its shell marks its commands.
        marking: bool,
        /// The user and folder that it reported last.
        state: Option<String>,
    },
    /// The terminal ended; nothing follows.
    Ended {
        /// How.
        ending: EndingView,
    },
}

/// A command's output as text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BlockTextView {
    /// The command line.
    pub command: String,
    /// The output's lines.
    pub lines: Vec<String>,
    /// How many lines are left out, which the text marks with `…`.
    pub omitted: u32,
}

/// Why a command or channel of the plugin failed. The host's refusals, such
/// as `plugin_off`, come beside these (ADR-0034, decision 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum TerminalError {
    /// The conversation has no terminal with this number.
    #[error("the conversation has no such terminal")]
    TerminalNotFound,
    /// The terminal ended.
    #[error("the terminal ended")]
    TerminalEnded,
    /// The terminal has no command with this number.
    #[error("the terminal has no such command")]
    BlockNotFound,
    /// The conversation does not exist.
    #[error("the conversation does not exist")]
    ConversationNotFound,
    /// The shell is not one of this computer's shells.
    #[error("the shell is not one of this computer's shells")]
    ShellNotListed,
    /// No plugin offers the host.
    #[error("no plugin offers this host")]
    HostNotFound,
    /// The host does not open terminals now, for its plugin's reason.
    #[error("the host does not open terminals now")]
    HostUnavailable {
        /// The host's plugin's code.
        reason: String,
    },
    /// The folder is not a folder that exists.
    #[error("the folder is not a folder that exists")]
    FolderInvalid,
    /// The conversation or this computer holds as many terminals as it may.
    #[error("as many terminals run as are allowed")]
    TooMany {
        /// The limit.
        limit: u32,
    },
    /// This system cannot run terminals.
    #[error("this system cannot run terminals")]
    Unsupported,
    /// The terminal could not be opened, for a reason that the log has.
    #[error("the terminal could not be opened")]
    OpenFailed {
        /// The cause's code.
        reason: String,
    },
    /// The settings could not be read or stored.
    #[error("the settings could not be read or stored")]
    StorageFailed,
}

impl PluginError for TerminalError {
    fn code(&self) -> &'static str {
        match self {
            Self::TerminalNotFound => "terminal.terminal_not_found",
            Self::TerminalEnded => "terminal.terminal_ended",
            Self::BlockNotFound => "terminal.block_not_found",
            Self::ConversationNotFound => "terminal.conversation_not_found",
            Self::ShellNotListed => "terminal.shell_not_listed",
            Self::HostNotFound => "terminal.host_not_found",
            Self::HostUnavailable { .. } => "terminal.host_unavailable",
            Self::FolderInvalid => "terminal.folder_invalid",
            Self::TooMany { .. } => "terminal.too_many",
            Self::Unsupported => "terminal.unsupported",
            Self::OpenFailed { .. } => "terminal.open_failed",
            Self::StorageFailed => "terminal.storage_failed",
        }
    }
}

/// The kind of a shell, as the plugin's texts name it.
pub(crate) fn shell_kind_name(kind: ShellKind) -> &'static str {
    match kind {
        ShellKind::Bash => "bash",
        ShellKind::Zsh => "zsh",
        ShellKind::Fish => "fish",
        ShellKind::Sh => "sh",
        ShellKind::PowerShell => "PowerShell",
        ShellKind::Cmd => "Command Prompt",
        _ => "shell",
    }
}

macro_rules! command {
    ($spec:ident, $name:literal, $docs:literal, $input:ty, $output:ty) => {
        #[doc = $docs]
        #[derive(Debug)]
        pub enum $spec {}

        impl CommandSpec for $spec {
            const NAME: &'static str = $name;
            const DOCS: &'static str = $docs;
            type Input = $input;
            type Output = $output;
            type Error = TerminalError;
        }
    };
}

macro_rules! channel {
    ($spec:ident, $name:literal, $docs:literal, $input:ty, $item:ty) => {
        #[doc = $docs]
        #[derive(Debug)]
        pub enum $spec {}

        impl ChannelSpec for $spec {
            const NAME: &'static str = $name;
            const DOCS: &'static str = $docs;
            type Input = $input;
            type Item = $item;
            type Error = TerminalError;
        }
    };
}

command!(
    OpenTerminalCommand,
    "open_terminal",
    "Opens a terminal of a conversation on this computer or on a host that a plugin offers, and returns it.",
    OpenTerminal,
    TerminalSummary
);

/// `write_terminal`: sends what the owner typed or pasted to the terminal,
/// as typed. Its input may be larger than the default, since JSON escapes
/// control characters.
#[derive(Debug)]
pub enum WriteTerminalCommand {}

impl CommandSpec for WriteTerminalCommand {
    const NAME: &'static str = "write_terminal";
    const DOCS: &'static str = "Sends what the owner typed or pasted to the terminal, as typed.";
    // Six bytes of JSON for each control character of the data, at most.
    const MAX_INPUT_BYTES: u32 = 6 * MAX_WRITE_BYTES + 1024;
    type Input = WriteTerminal;
    type Output = ();
    type Error = TerminalError;
}

command!(
    ResizeTerminalCommand,
    "resize_terminal",
    "Changes a terminal's size to its view's.",
    ResizeTerminal,
    ()
);
command!(
    EndTerminalCommand,
    "end_terminal",
    "Ends a terminal and everything that it started, or removes one that ended.",
    TerminalRef,
    ()
);
command!(
    AcknowledgeOutputCommand,
    "acknowledge_output",
    "Tells the terminal how much of its updates a view has written, so that it sends more.",
    AcknowledgeOutput,
    ()
);
command!(
    BlockTextCommand,
    "block_text",
    "The output of a command of a terminal, as text.",
    BlockRef,
    BlockTextView
);
command!(
    MentionBlockCommand,
    "mention_block",
    "The mention of a command of a terminal, which the view puts into the message box.",
    BlockRef,
    MentionEntry
);

/// `mention_selection`: keeps text that the owner selected in a terminal and
/// returns its mention, which the view puts into the message box.
#[derive(Debug)]
pub enum MentionSelectionCommand {}

impl CommandSpec for MentionSelectionCommand {
    const NAME: &'static str = "mention_selection";
    const DOCS: &'static str = "Keeps text that the owner selected in a terminal and returns its mention, which the view puts into the message box.";
    const MAX_INPUT_BYTES: u32 = 6 * MAX_SELECTION_BYTES + 1024;
    type Input = MentionSelection;
    type Output = MentionEntry;
    type Error = TerminalError;
}

command!(
    GetSettingsCommand,
    "get_settings",
    "The plugin's settings.",
    (),
    TerminalSettings
);
command!(
    UpdateSettingsCommand,
    "update_settings",
    "Stores the plugin's settings and returns what it stored.",
    TerminalSettings,
    TerminalSettings
);
channel!(
    FollowHostsChannel,
    "follow_hosts",
    "Where terminals open: this computer's shells and the hosts that plugins offer, each item the complete list.",
    (),
    HostsView
);
channel!(
    FollowTerminalsChannel,
    "follow_terminals",
    "The terminals of a conversation, each item the complete list.",
    ConversationRef,
    TerminalListUpdate
);
channel!(
    FollowTerminalChannel,
    "follow_terminal",
    "What a view of a terminal shows: a snapshot first, then output, commands and state in order.",
    FollowTerminal,
    TerminalUpdate
);

/// The plugin's commands, its channels and the limits that the UI checks.
pub fn interface() -> Interface {
    Interface::new()
        .command::<OpenTerminalCommand>()
        .command::<WriteTerminalCommand>()
        .command::<ResizeTerminalCommand>()
        .command::<EndTerminalCommand>()
        .command::<AcknowledgeOutputCommand>()
        .command::<BlockTextCommand>()
        .command::<MentionBlockCommand>()
        .command::<MentionSelectionCommand>()
        .command::<GetSettingsCommand>()
        .command::<UpdateSettingsCommand>()
        .channel::<FollowHostsChannel>()
        .channel::<FollowTerminalsChannel>()
        .channel::<FollowTerminalChannel>()
        .constant(
            "MAX_WRITE_BYTES",
            "The most bytes of one write to a terminal.",
            MAX_WRITE_BYTES,
        )
        .constant(
            "MAX_SELECTION_BYTES",
            "The most bytes of a selection that the owner mentions.",
            MAX_SELECTION_BYTES,
        )
        .constant(
            "MIN_SCROLLBACK",
            "The fewest lines of scrollback.",
            MIN_SCROLLBACK,
        )
        .constant(
            "MAX_SCROLLBACK",
            "The most lines of scrollback.",
            MAX_SCROLLBACK,
        )
        .constant(
            "MIN_FONT_SIZE",
            "The smallest font size, in pixels.",
            MIN_FONT_SIZE,
        )
        .constant(
            "MAX_FONT_SIZE",
            "The largest font size, in pixels.",
            MAX_FONT_SIZE,
        )
        .constant(
            "MAX_COLUMNS",
            "The most columns of a terminal.",
            u32::from(MAX_TERMINAL_COLUMNS),
        )
        .constant(
            "MAX_ROWS",
            "The most rows of a terminal.",
            u32::from(MAX_TERMINAL_ROWS),
        )
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::ConversationId;
    use mannie_plugin_sdk::ipc::Validate;

    use super::{
        FollowTerminal, HostChoice, MentionSelection, OpenTerminal, TerminalSettings, WriteTerminal,
    };

    fn conversation() -> ConversationId {
        ConversationId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap()
    }

    #[test]
    fn inputs_keep_their_limits() {
        let open = |columns, rows, folder: Option<&str>| OpenTerminal {
            conversation_id: conversation(),
            host: HostChoice::ThisComputer { shell: None },
            folder: folder.map(str::to_owned),
            columns,
            rows,
        };
        open(120, 30, Some("/home/me")).validate().unwrap();
        assert_eq!(open(0, 30, None).validate().unwrap_err().field, "columns");
        assert_eq!(
            open(120, 501, None).validate().unwrap_err().field,
            "columns"
        );
        assert_eq!(
            open(120, 30, Some(" ")).validate().unwrap_err().field,
            "folder"
        );

        let write = |data: &str| WriteTerminal {
            conversation_id: conversation(),
            terminal: 1,
            data: data.to_owned(),
        };
        write("ls\r").validate().unwrap();
        assert!(write("").validate().is_err());
        assert!(write(&"x".repeat(64 * 1024 + 1)).validate().is_err());

        let follow = |view: &str, scrollback| FollowTerminal {
            conversation_id: conversation(),
            terminal: 1,
            view: view.to_owned(),
            scrollback,
        };
        follow("0b6c-41", 2000).validate().unwrap();
        assert_eq!(follow("a b", 2000).validate().unwrap_err().field, "view");
        assert_eq!(follow("v", 100).validate().unwrap_err().field, "scrollback");

        let selection = MentionSelection {
            conversation_id: conversation(),
            terminal: 1,
            text: "  ".to_owned(),
        };
        assert_eq!(selection.validate().unwrap_err().field, "text");
    }

    #[test]
    fn the_settings_keep_their_ranges() {
        TerminalSettings::default().validate().unwrap();
        let small = TerminalSettings {
            scrollback: 100,
            ..TerminalSettings::default()
        };
        assert_eq!(small.validate().unwrap_err().field, "scrollback");
        let large_font = TerminalSettings {
            font_size: 40,
            ..TerminalSettings::default()
        };
        assert_eq!(large_font.validate().unwrap_err().field, "fontSize");
    }
}
