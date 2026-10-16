//! The words of a command, which grants and rules match
//! (docs/features/system-tools/operations-and-approvals.md, rules).
//!
//! A rule allows commands that start with its words. That is safe only when
//! the words Mannie sees are the words the shell runs, so a command with
//! anything that can start a second command, substitute text or redirect
//! output is never covered: its approval offers only "Allow once" and "Deny".

/// Characters that make a command impossible to cover: they separate
/// commands, substitute text, group, redirect or escape in `sh` or in
/// PowerShell. They count anywhere in the command, quoted or not.
const METACHARACTERS: [char; 13] = [
    ';', '&', '|', '$', '`', '(', ')', '{', '}', '<', '>', '\n', '\r',
];

/// The typographic quotes that PowerShell treats as quotes.
const TYPOGRAPHIC_QUOTES: [char; 7] = [
    '\u{2018}', '\u{2019}', '\u{201a}', '\u{201b}', '\u{201c}', '\u{201d}', '\u{201e}',
];

/// Whether a character keeps a command from being covered.
fn is_refused(character: char) -> bool {
    if METACHARACTERS.contains(&character) || TYPOGRAPHIC_QUOTES.contains(&character) {
        return true;
    }
    // `sh` escapes with a backslash, which Mannie does not interpret. On
    // Windows a backslash separates the names of a path.
    if cfg!(unix) && character == '\\' {
        return true;
    }
    // Words are split only at spaces and tabs; any other white space or
    // control character could split them differently in the shell.
    (character.is_whitespace() || character.is_control()) && character != ' ' && character != '\t'
}

/// The words of a command, split at spaces and tabs outside quotes, with the
/// quotes removed. `None` when a rule or grant can never cover the command:
/// it contains a metacharacter, an unusual space or an unbalanced quote, or
/// it has no words. A worker reads its host rules with it too.
pub fn command_words(command: &str) -> Option<Vec<String>> {
    if command.chars().any(is_refused) {
        return None;
    }
    let mut words = Vec::new();
    let mut word = String::new();
    // Whether the current word has started, so `""` is an empty word.
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for character in command.chars() {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (Some(_), _) => word.push(character),
            (None, '\'' | '"') => {
                quote = Some(character);
                in_word = true;
            }
            (None, ' ' | '\t') => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (None, _) => {
                word.push(character);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if in_word {
        words.push(word);
    }
    if words.is_empty() {
        return None;
    }
    Some(words)
}

/// Whether a rule's words start the command's words.
pub(crate) fn starts_with_words(command: &[String], rule: &[String]) -> bool {
    !rule.is_empty() && command.starts_with(rule)
}

#[cfg(test)]
mod tests {
    use super::{command_words, starts_with_words};

    fn words(command: &str) -> Option<Vec<String>> {
        command_words(command)
    }

    fn list(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn plain_commands_split_into_words() {
        assert_eq!(words("git status"), Some(list(&["git", "status"])));
        assert_eq!(
            words("  cargo\ttest  -p   mannie-tools "),
            Some(list(&["cargo", "test", "-p", "mannie-tools"]))
        );
        assert_eq!(
            words(r#"git commit -m "fix the parser""#),
            Some(list(&["git", "commit", "-m", "fix the parser"]))
        );
        assert_eq!(words("echo '' done"), Some(list(&["echo", "", "done"])));
        assert_eq!(
            words(r#"npm run "test:unit""#),
            Some(list(&["npm", "run", "test:unit"]))
        );
        assert_eq!(
            words("ls -la ~/work/*.rs"),
            Some(list(&["ls", "-la", "~/work/*.rs"]))
        );
        assert_eq!(
            words("git log -n 5 # recent"),
            Some(list(&["git", "log", "-n", "5", "#", "recent"]))
        );
    }

    #[test]
    fn commands_with_metacharacters_are_never_covered() {
        // A corpus of ways to run a second command, substitute text or
        // redirect output in sh and in PowerShell.
        let corpus = [
            "git status; rm -rf work",
            "git status && rm -rf work",
            "git status || rm -rf work",
            "git status & calc",
            "git status | sh",
            "git status `rm -rf work`",
            "git status $(rm -rf work)",
            "git status ${HOME}",
            "git status $HOME",
            "git status > out.txt",
            "git status >> out.txt",
            "git status < in.txt",
            "git status 2>&1",
            "git status\nrm -rf work",
            "git status\r\nrm -rf work",
            "git status (Remove-Item work)",
            "git status @(Remove-Item work)",
            "git status { Remove-Item work }",
            "& 'C:\\tools\\evil.exe'",
            "git status; Remove-Item -Recurse work",
            "git status `; Remove-Item work",
            "git \"status; rm -rf work\"",
            "git 'status $(rm -rf work)'",
            "git status \u{2018}quoted\u{2019}",
            "git status \u{201c}quoted\u{201d}",
            "git\u{a0}status",
            "git\u{2003}status",
            "git\u{b}status",
            "git status \"unbalanced",
            "git status 'unbalanced",
            "",
            "   ",
        ];
        for command in corpus {
            assert_eq!(command_words(command), None, "{command:?}");
        }
    }

    #[test]
    fn an_empty_quoted_word_is_a_word() {
        assert_eq!(words("''"), Some(list(&[""])));
        assert!(!starts_with_words(
            &words("''").unwrap(),
            &["git".to_owned()]
        ));
    }

    #[cfg(unix)]
    #[test]
    fn backslashes_are_escapes_in_sh() {
        assert_eq!(command_words(r"git\ status"), None);
        assert_eq!(command_words(r"l\s"), None);
    }

    #[cfg(windows)]
    #[test]
    fn backslashes_are_separators_on_windows() {
        assert_eq!(
            command_words(r".\build.ps1 -Release"),
            Some(list(&[r".\build.ps1", "-Release"]))
        );
    }

    #[test]
    fn rules_match_leading_whole_words() {
        let command = words("git status --short").unwrap();
        let rule = |text: &str| words(text).unwrap();

        assert!(starts_with_words(&command, &rule("git")));
        assert!(starts_with_words(&command, &rule("git status")));
        assert!(starts_with_words(&command, &rule("git status --short")));
        assert!(!starts_with_words(&command, &rule("git stat")));
        assert!(!starts_with_words(
            &command,
            &rule("git status --short --branch")
        ));
        assert!(!starts_with_words(&command, &rule("Git")));
        assert!(!starts_with_words(&command, &[]));
    }
}
