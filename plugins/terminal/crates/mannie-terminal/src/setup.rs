//! The setups that make shells mark their prompts and commands
//! (plugins/terminal/docs/adr/0001-mark-commands-with-shell-integration.md).
//!
//! A shell of this computer gets its setup at start, after the owner's
//! profile, through the core (ADR-0053, decision 1). A remote host's login
//! shell gets it typed after its first prompt, as one line that starts with
//! a space, which most shells keep out of their history, and that clears the
//! screen at its end, so its echo does not stay.
//!
//! No setup starts a process for a prompt or a command: bash marks through
//! `PS0` and `PROMPT_COMMAND` with builtins only, never through a `DEBUG`
//! trap; zsh through `precmd` and `preexec`; fish through its events;
//! PowerShell through its prompt function and `PSReadLine`'s `ReadLine`; sh
//! and cmd through their prompt variables, which end a command without a
//! start marker or an exit code from cmd. Values escape `\` and `;` where the
//! shell can without a process.

use mannie_plugin_sdk::services::terminals::ShellKind;

/// The marker that each setup's text carries in place of the nonce.
const NONCE: &str = "@NONCE@";

/// bash: `D` and the properties first in `PROMPT_COMMAND`, so `$?` is the
/// command's; `A` and `B` around the prompt last, after a framework such as
/// starship rewrote it; `C` from `PS0`, whose arithmetic sets the flag in the
/// shell itself (the substring of length 0 is empty).
const BASH: &str = r#"__mannie_n='@NONCE@'
__mannie_ran=
__mannie_report() {
  local code=$?
  if [ -n "$__mannie_ran" ]; then
    printf '\033]633;D;%s;%s\007' "$code" "$__mannie_n"
    __mannie_ran=
  fi
  local cwd=${PWD//\\/\\\\}
  cwd=${cwd//;/\\x3b}
  printf '\033]633;P;Cwd=%s;%s\007\033]633;P;User=%s;%s\007' "$cwd" "$__mannie_n" "${USER:-$LOGNAME}" "$__mannie_n"
  return $code
}
__mannie_wrap() {
  local code=$?
  case $PS1 in
    *"633;B;$__mannie_n"*) ;;
    *) PS1='\[\033]633;A;'"$__mannie_n"'\007\]'"$PS1"'\[\033]633;B;'"$__mannie_n"'\007\]' ;;
  esac
  return $code
}
if (( BASH_VERSINFO[0] > 5 || (BASH_VERSINFO[0] == 5 && BASH_VERSINFO[1] >= 1) )); then
  PROMPT_COMMAND=(__mannie_report "${PROMPT_COMMAND[@]}" __mannie_wrap)
else
  PROMPT_COMMAND="__mannie_report${PROMPT_COMMAND:+;$PROMPT_COMMAND};__mannie_wrap"
fi
PS0='\033]633;C;'"$__mannie_n"'\007${__mannie_n:0:$((__mannie_ran=1,0))}'"$PS0"
"#;

/// zsh: the report first among the `precmd` functions, so `$?` is the
/// command's, and the prompt's markers last, after a theme rewrote it.
const ZSH: &str = r#"__mannie_n='@NONCE@'
typeset -g __mannie_ran=
__mannie_report() {
  local code=$?
  if [[ -n $__mannie_ran ]]; then
    printf '\033]633;D;%s;%s\007' $code $__mannie_n
    __mannie_ran=
  fi
  local cwd=${PWD//\\/\\\\}
  cwd=${cwd//;/\\x3b}
  printf '\033]633;P;Cwd=%s;%s\007\033]633;P;User=%s;%s\007' "$cwd" $__mannie_n "$USERNAME" $__mannie_n
  return $code
}
__mannie_wrap() {
  if [[ $PS1 != *"633;B;$__mannie_n"* ]]; then
    PS1=$'%{\e]633;A;'$__mannie_n$'\a%}'$PS1$'%{\e]633;B;'$__mannie_n$'\a%}'
  fi
}
__mannie_preexec() {
  __mannie_ran=1
  local line=${1//\\/\\\\}
  line=${line//;/\\x3b}
  line=${line//$'\n'/\\x0a}
  printf '\033]633;E;%s;%s\007\033]633;C;%s\007' "$line" $__mannie_n $__mannie_n
}
precmd_functions=(__mannie_report $precmd_functions __mannie_wrap)
preexec_functions+=(__mannie_preexec)
"#;

/// fish: its events give the command line, its status and the prompt; the
/// prompt function keeps the owner's and adds `B`.
const FISH: &str = r"set -g __mannie_n '@NONCE@'
function __mannie_escape
    string replace -a -- '\\' '\\\\' $argv | string replace -a -- ';' '\\x3b' | string join -- '\\x0a'
end
function __mannie_preexec --on-event fish_preexec
    set -g __mannie_ran 1
    printf '\e]633;E;%s;%s\a\e]633;C;%s\a' (__mannie_escape $argv) $__mannie_n $__mannie_n
end
function __mannie_postexec --on-event fish_postexec
    set -l code $status
    set -e __mannie_ran
    printf '\e]633;D;%s;%s\a' $code $__mannie_n
end
function __mannie_prompt_start --on-event fish_prompt
    printf '\e]633;P;Cwd=%s;%s\a\e]633;P;User=%s;%s\a\e]633;A;%s\a' (__mannie_escape $PWD) $__mannie_n $USER $__mannie_n $__mannie_n
end
functions -c fish_prompt __mannie_owner_prompt
function fish_prompt
    __mannie_owner_prompt
    printf '\e]633;B;%s\a' $__mannie_n
end
";

/// sh, such as dash and FreeBSD's sh: the prompt variable, which the shell
/// expands at each prompt, ends the last command with its status. The two
/// control characters come from `printf` once, at the setup.
const SH: &str = r#"__mannie_n='@NONCE@'
__mannie_e=$(printf '\033')
__mannie_b=$(printf '\007')
PS1="${__mannie_e}]633;D;\$?;${__mannie_n}${__mannie_b}${__mannie_e}]633;P;Cwd=\${PWD};${__mannie_n}${__mannie_b}${__mannie_e}]633;A;${__mannie_n}${__mannie_b}${PS1:-\$ }${__mannie_e}]633;B;${__mannie_n}${__mannie_b}"
"#;

/// PowerShell, as one line without double quotes, which travels as the
/// shell's `-Command`. `PSReadLine`'s `ReadLine` gives the command line and
/// `C`. The exit code comes from `$?`, or from `$LASTEXITCODE` when the
/// command changed it, since an interrupted command leaves the old value.
const POWERSHELL: &[&str] = &[
    "Import-Module PSReadLine -ErrorAction SilentlyContinue",
    "$global:__mn='@NONCE@'",
    "$global:__me=[char]27",
    "$global:__mb=[char]7",
    "$global:__mran=$false",
    "$global:__mlast=$global:LASTEXITCODE",
    "$global:__mprompt=$function:prompt",
    "function global:__mesc([string]$v) { $v.Replace('\\','\\\\').Replace(';','\\x3b').Replace([string][char]10,'\\x0a').Replace([string][char]13,'\\x0d') }",
    "function global:prompt { $ok=$global:?; $code=$global:LASTEXITCODE; if ($ok) { $exit=0 } elseif (($null -ne $code) -and ($code -ne $global:__mlast)) { $exit=$code } else { $exit=1 }; $s=''; if ($global:__mran -or -not $global:__mreadline) { $s += $global:__me + ']633;D;' + $exit + ';' + $global:__mn + $global:__mb; $global:__mran=$false }; $s += $global:__me + ']633;P;Cwd=' + (__mesc $PWD.ProviderPath) + ';' + $global:__mn + $global:__mb + $global:__me + ']633;P;User=' + (__mesc ([Environment]::UserName)) + ';' + $global:__mn + $global:__mb + $global:__me + ']633;A;' + $global:__mn + $global:__mb; $p = & $global:__mprompt; $global:LASTEXITCODE=$code; $s + $p + $global:__me + ']633;B;' + $global:__mn + $global:__mb }",
    "$global:__mreadline=$function:PSConsoleHostReadLine",
    "if ($global:__mreadline) { function global:PSConsoleHostReadLine { $line = & $global:__mreadline; $global:__mlast=$global:LASTEXITCODE; [Console]::Write($global:__me + ']633;E;' + (__mesc $line) + ';' + $global:__mn + $global:__mb + $global:__me + ']633;C;' + $global:__mn + $global:__mb); $global:__mran=$true; $line } }",
];

/// cmd: the prompt variable, whose `$E` is the escape character; `$E\`
/// ends each marker. cmd gives no exit code and no command line.
const CMD: &str =
    "$E]633;D;;@NONCE@$E\\$E]633;P;Cwd=$P;@NONCE@$E\\$E]633;A;@NONCE@$E\\$P$G$E]633;B;@NONCE@$E\\";

/// The setup that a shell of this computer runs after the owner's profile,
/// or `None` for a shell that no setup reaches.
pub(crate) fn injected(kind: ShellKind, nonce: &str) -> Option<String> {
    let text = match kind {
        ShellKind::Bash => BASH.to_owned(),
        ShellKind::Zsh => ZSH.to_owned(),
        ShellKind::Fish => FISH.to_owned(),
        ShellKind::Sh => SH.to_owned(),
        ShellKind::PowerShell => POWERSHELL.join("; "),
        ShellKind::Cmd => CMD.to_owned(),
        _ => return None,
    };
    Some(text.replace(NONCE, nonce))
}

/// What the plugin types into a remote host's login shell after its first
/// prompt: one line that starts with a space and ends with Enter, or `None`
/// for a shell that no setup reaches.
pub(crate) fn typed(kind: ShellKind, nonce: &str) -> Option<String> {
    let clear = "printf '\\033[H\\033[2J'";
    let line = match kind {
        ShellKind::Bash | ShellKind::Zsh => {
            let script = format!("{}{clear}\n", injected(kind, nonce)?);
            format!(" eval {}", ansi_c_quoted(&script))
        }
        ShellKind::Sh => {
            let script = format!("{}{clear}\n", injected(kind, nonce)?);
            format!(
                " eval \"$(printf '%b' {})\"",
                sh_single_quoted(&octal_escaped(&script))
            )
        }
        ShellKind::Fish => {
            let script = format!("{}{clear}\n", injected(kind, nonce)?);
            let lines: Vec<String> = script.lines().map(fish_single_quoted).collect();
            format!(" printf '%s\\n' {} | source", lines.join(" "))
        }
        ShellKind::PowerShell => {
            format!(" {}; Clear-Host", injected(kind, nonce)?)
        }
        _ => return None,
    };
    Some(format!("{line}\r"))
}

/// A text as bash's and zsh's `$'...'`, which carries line breaks.
fn ansi_c_quoted(text: &str) -> String {
    let mut quoted = String::from("$'");
    for character in text.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '\'' => quoted.push_str("\\'"),
            '\n' => quoted.push_str("\\n"),
            '\x1b' => quoted.push_str("\\e"),
            '\x07' => quoted.push_str("\\a"),
            _ => quoted.push(character),
        }
    }
    quoted.push('\'');
    quoted
}

/// A text with its backslashes and line breaks as the escapes of `printf
/// '%b'`, which dash and FreeBSD's sh know.
fn octal_escaped(text: &str) -> String {
    let mut escaped = String::new();
    for character in text.chars() {
        match character {
            '\\' => escaped.push_str("\\0134"),
            '\n' => escaped.push_str("\\n"),
            _ => escaped.push(character),
        }
    }
    escaped
}

/// A text in sh's single quotes, which end and start again around a quote.
fn sh_single_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// A text in fish's single quotes, where only `\` and `'` are escaped.
fn fish_single_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'"))
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::services::terminals::ShellKind;

    use super::{injected, typed};

    const NONCE: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn every_known_shell_gets_a_setup_with_the_nonce() {
        for kind in [
            ShellKind::Bash,
            ShellKind::Zsh,
            ShellKind::Fish,
            ShellKind::Sh,
            ShellKind::PowerShell,
            ShellKind::Cmd,
        ] {
            let setup = injected(kind, NONCE).unwrap();
            assert!(setup.contains(NONCE), "{kind:?}");
            assert!(!setup.contains("@NONCE@"), "{kind:?}");
            assert!(setup.len() <= 16 * 1024, "{kind:?}");
        }
        assert_eq!(injected(ShellKind::Other, NONCE), None);
    }

    #[test]
    fn the_one_line_setups_fit_their_shells() {
        // PowerShell's travels in -Command, cmd's in PROMPT.
        let powershell = injected(ShellKind::PowerShell, NONCE).unwrap();
        assert!(!powershell.contains(['"', '\r', '\n']));
        let cmd = injected(ShellKind::Cmd, NONCE).unwrap();
        assert!(!cmd.contains(['\r', '\n']));
        assert!(cmd.starts_with("$E]633;D;;"));
    }

    #[test]
    fn no_setup_forks_for_a_prompt() {
        // Arithmetic expansion, `$((`, runs in the shell; command
        // substitution, `$(`, would fork.
        let bash = injected(ShellKind::Bash, NONCE).unwrap();
        assert!(!bash.contains("trap"));
        assert!(!bash.replace("$((", "").contains("$("));
        let zsh = injected(ShellKind::Zsh, NONCE).unwrap();
        assert!(!zsh.replace("$((", "").contains("$("));
    }

    #[test]
    fn a_typed_setup_is_one_line_that_starts_with_a_space() {
        for kind in [
            ShellKind::Bash,
            ShellKind::Zsh,
            ShellKind::Sh,
            ShellKind::Fish,
            ShellKind::PowerShell,
        ] {
            let line = typed(kind, NONCE).unwrap();
            assert!(line.starts_with(' '), "{kind:?}");
            assert!(line.ends_with('\r'), "{kind:?}");
            assert_eq!(line.matches(['\r', '\n']).count(), 1, "{kind:?}");
        }
        assert_eq!(typed(ShellKind::Cmd, NONCE), None);
    }
}
