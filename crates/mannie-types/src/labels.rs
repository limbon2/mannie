//! Short texts that a model writes for the owner, such as a tool call's
//! activity or a task's title, as the owner reads them. The tools, the
//! runtime and the plugins' executors clean them the same way.

/// A short text that a model wrote for the owner, such as an activity or a
/// task's title, as the owner reads it: without control characters and
/// invisible characters, with its white space collapsed to single spaces,
/// and at most `max_chars` characters, the last of them "…" when it was
/// longer. `None` when nothing is left.
pub fn clean_label(text: &str, max_chars: usize) -> Option<String> {
    let shown: String = text
        .chars()
        .filter(|character| !is_invisible(*character))
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    let joined = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.is_empty() {
        return None;
    }
    if joined.chars().count() <= max_chars {
        return Some(joined);
    }
    let kept: String = joined.chars().take(max_chars.saturating_sub(1)).collect();
    Some(format!("{}…", kept.trim_end()))
}

/// Characters that change how the text around them shows without showing
/// themselves: the controls of bidirectional text, which could make a label
/// read in another order than it was written, and zero-width spaces. The
/// zero-width joiners stay, because some scripts and emoji need them.
fn is_invisible(character: char) -> bool {
    matches!(
        character,
        '\u{061C}'
            | '\u{200B}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

#[cfg(test)]
mod tests {
    use super::clean_label;

    #[test]
    fn a_label_reads_as_one_line_of_visible_text() {
        assert_eq!(
            clean_label("  Reading\n\tthe   notes\u{7} ", 100).as_deref(),
            Some("Reading the notes")
        );
        // A right-to-left override would show "Reading txt.exe" as "Reading exe.txt".
        assert_eq!(
            clean_label("Reading \u{202E}exe.txt\u{202C}", 100).as_deref(),
            Some("Reading exe.txt")
        );
        assert_eq!(
            clean_label("Read\u{200B}ing\u{FEFF} notes", 100).as_deref(),
            Some("Reading notes")
        );
        // Some scripts need the zero-width joiners.
        assert_eq!(
            clean_label("می\u{200C}خوانم", 100).as_deref(),
            Some("می\u{200C}خوانم")
        );
    }

    #[test]
    fn a_label_without_visible_text_is_none() {
        for text in ["", "   ", "\u{202E}\u{200B}", "\n\t"] {
            assert_eq!(clean_label(text, 100), None, "{text:?}");
        }
    }

    #[test]
    fn a_long_label_is_cut_at_a_character() {
        let label = clean_label(&"é".repeat(20), 10).unwrap();

        assert_eq!(label.chars().count(), 10);
        assert!(label.ends_with('…'), "{label}");
        assert_eq!(clean_label(&"a".repeat(10), 10), Some("a".repeat(10)));
    }
}
