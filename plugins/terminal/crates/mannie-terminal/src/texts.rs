//! What the model reads of the plugin: the tools' descriptions, their
//! results and refusals, and the instruction section. The owner reads the
//! plugin through its UI, whose text comes from the message catalog.

use std::fmt::Write as _;

/// The description of `run_in_terminal`.
pub(crate) const RUN_DESCRIPTION: &str = "Runs one command in a terminal of this conversation, \
which the owner sees beside the conversation, and returns its output with numbered lines. Use a \
terminal for commands that the owner should watch, or that need a terminal's state: its folder, \
its environment, its SSH session or a program that asks questions. You type at an idle prompt \
only, never while the owner types or a command runs; a command that is still running when the \
wait ends is interrupted with Ctrl+C. Without a terminal number, your own idle terminal on the \
host is used, or a new one opens.";

/// The description of `list_terminals`.
pub(crate) const LIST_DESCRIPTION: &str = "Lists this conversation's terminals, with their \
hosts, shells and states, and the hosts where a new terminal opens. It never shows what the owner \
typed.";

/// A refusal of the arguments.
pub(crate) fn invalid_arguments(detail: &str) -> String {
    format!("The arguments are not valid: {detail}")
}

/// A blank command.
pub(crate) const BLANK_COMMAND: &str = "The command is blank.";

/// A command longer than the limit.
pub(crate) const LONG_COMMAND: &str = "The command is longer than 16 KiB.";

/// A command with a line break or a control character.
pub(crate) const COMMAND_NOT_ONE_LINE: &str = "The command must be one line without control characters; join several commands as the shell does, such as with `;` or `&&`.";

/// A timeout outside its range.
pub(crate) const TIMEOUT_OUT_OF_RANGE: &str = "timeout_seconds must be from 1 to 600.";

/// A terminal number that the conversation does not have.
pub(crate) fn unknown_terminal(number: u32) -> String {
    format!("This conversation has no terminal {number}. list_terminals lists them.")
}

/// A terminal that ended.
pub(crate) fn ended_terminal(number: u32) -> String {
    format!("Terminal {number} ended. Leave out the terminal number to use a new one.")
}

/// A terminal whose shell does not mark commands.
pub(crate) fn plain_terminal(number: u32) -> String {
    format!(
        "Terminal {number} runs a shell that does not mark its commands, so commands cannot run there. Use another terminal, or leave out the terminal number."
    )
}

/// A host that the arguments name and nobody offers.
pub(crate) fn unknown_host(name: &str, hosts: &[String]) -> String {
    if hosts.is_empty() {
        return format!("There is no host named {name}. Terminals open on this computer only.");
    }
    format!(
        "There is no host named {name}. Terminals open on this computer or on {}.",
        hosts.join(", ")
    )
}

/// A host that offers no terminals now.
pub(crate) fn unavailable_host(name: &str, code: &str) -> String {
    format!("Terminals do not open on {name} now ({code}).")
}

/// A shell that this computer does not have.
pub(crate) fn unknown_shell(name: &str, shells: &[String]) -> String {
    format!(
        "This computer has no shell named {name}. Its shells are: {}.",
        shells.join(", ")
    )
}

/// The shell of a remote host whose plugin has not named its login shell.
pub(crate) const LOGIN_SHELL: &str = "login shell";

/// A shell named for a remote host, whose login shell runs.
pub(crate) const SHELL_ON_REMOTE: &str =
    "A shell can be chosen only on this computer; a remote host runs its login shell.";

/// A call that did not run, with why.
pub(crate) fn not_run(reason: &str) -> String {
    format!("The command did not run. {reason}")
}

/// The terminal or host changed since the approval.
pub(crate) const CHANGED_SINCE_APPROVAL: &str = "The terminal or its host changed since the approval. Call the tool again, so the owner can decide on it as it is now.";

/// A terminal whose user or folder changed since the approval.
pub(crate) fn state_changed(number: u32, now: Option<&str>) -> String {
    match now {
        Some(now) => format!(
            "Terminal {number} is now {now}, not as the approval named it. Call the tool again, so the owner can decide on it as it is now."
        ),
        None => format!("Terminal {number} changed since the approval. Call the tool again."),
    }
}

/// A terminal that runs a command.
pub(crate) fn busy_terminal(number: u32) -> String {
    format!(
        "A command runs in terminal {number}, or its shell has not reached its prompt. Wait for it, or leave out the terminal number to use another one."
    )
}

/// A terminal where the owner types.
pub(crate) fn owner_typing(number: u32) -> String {
    format!(
        "The owner is typing in terminal {number}. Try again later, or leave out the terminal number to use another one."
    )
}

/// A new terminal whose shell never reached a marked prompt.
pub(crate) fn no_prompt(number: u32) -> String {
    format!(
        "Terminal {number} opened, but its shell did not reach a prompt that marks commands, so the command was not typed. The owner can use the terminal."
    )
}

/// A terminal that could not be opened.
pub(crate) fn open_failed(code: &str) -> String {
    format!("The terminal could not be opened ({code}).")
}

/// A command that could not be typed.
pub(crate) const NOT_TYPED: &str = "The command could not be typed, because the terminal ended.";

/// How a command ended, as the result's first line says it.
pub(crate) fn ran(
    command: &str,
    number: u32,
    place: &str,
    folder: Option<&str>,
    ending: &str,
) -> String {
    let mut text = format!("Ran `{command}` in terminal {number} ({place})");
    if let Some(folder) = folder {
        let _written = write!(text, ", in {folder}");
    }
    let _written = write!(text, ". {ending}");
    text
}

/// A command that exited.
pub(crate) fn exited(code: i32, duration: &str) -> String {
    format!("It exited with {code} after {duration}.")
}

/// A command that ended without an exit code, as in cmd.
pub(crate) fn finished(duration: &str) -> String {
    format!("It ended after {duration}; the shell gives no exit code.")
}

/// A command that the timeout interrupted.
pub(crate) fn interrupted(timeout_seconds: u32) -> String {
    format!("It was still running after {timeout_seconds} s, so it was interrupted with Ctrl+C.")
}

/// A command that runs on after Ctrl+C.
pub(crate) fn still_running(number: u32, timeout_seconds: u32) -> String {
    format!(
        "The command was still running after {timeout_seconds} s and did not end after Ctrl+C. It may still run in terminal {number}, which no command can use until it ends."
    )
}

/// How to cite the output.
pub(crate) fn cite_hint(link: &str, key: &str) -> String {
    format!(
        "Cite its output as {link}, or lines of it with a fragment, such as [lines 3-4](cite:terminal/{key}#L3-L4)."
    )
}

/// An output that printed nothing.
pub(crate) const NO_OUTPUT: &str = "It printed nothing.";

/// The heading of the numbered output.
pub(crate) fn output_heading(lines: usize, omitted: u64) -> String {
    if omitted == 0 {
        return format!("Output ({lines} lines):");
    }
    format!("Output ({lines} lines; {omitted} lines are left out where `…` stands):")
}

/// The note of an output that did not fit.
pub(crate) fn output_archived(archive: &str) -> String {
    format!(
        "The output did not fit; its start and end are shown. read_tool_output reads all of it from the archive {archive}."
    )
}

/// A duration as the model reads it.
pub(crate) fn duration(milliseconds: u64) -> String {
    if milliseconds < 1000 {
        return format!("{milliseconds} ms");
    }
    let seconds = milliseconds / 1000;
    let tenths = (milliseconds % 1000) / 100;
    if seconds < 60 {
        return format!("{seconds}.{tenths} s");
    }
    format!("{} min {} s", seconds / 60, seconds % 60)
}

/// The introduction of the instruction section.
pub(crate) const SECTION: &str = "Terminals: the conversation can have terminals beside it, \
which the owner sees and types in. run_in_terminal runs a command in one of them, and \
list_terminals lists them. Use a terminal for commands that the owner should watch, or that need \
a terminal's state, such as its folder, its environment or its SSH session; run_command remains \
right for commands that only you need. You read only the output of your own commands and what \
the owner mentions. To point the owner at output, copy its cite link from the result, and add \
#L<first>-L<last> to cite lines, such as [the listener](cite:terminal/3f9a2c1d.5#L3-L4).";

/// The section's line of where terminals open.
pub(crate) fn section_hosts(shells: &[String], hosts: &[String]) -> String {
    let mut text = format!(
        "New terminals open on this computer ({}",
        if shells.is_empty() {
            "no shell found".to_owned()
        } else {
            shells.join(", ")
        }
    );
    text.push(')');
    if hosts.is_empty() {
        text.push('.');
    } else {
        let _written = write!(text, " and on the hosts {}.", hosts.join(", "));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{SECTION, duration, section_hosts};

    #[test]
    fn durations_read_as_a_person_says_them() {
        assert_eq!(duration(250), "250 ms");
        assert_eq!(duration(1250), "1.2 s");
        assert_eq!(duration(125_000), "2 min 5 s");
    }

    #[test]
    fn the_section_stays_short() {
        let hosts = section_hosts(
            &["bash".to_owned(), "zsh".to_owned()],
            &["build-box".to_owned()],
        );
        assert!(SECTION.len() + hosts.len() < 1500);
        assert!(hosts.ends_with("and on the hosts build-box."));
    }
}
