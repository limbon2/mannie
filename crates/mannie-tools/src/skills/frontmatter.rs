//! The frontmatter of a `SKILL.md` (ADR-0061, decision 4; docs/features/
//! skills.md, the frontmatter): the YAML between a first line `---` and the
//! next line `---` or `...`.
//!
//! Skills written for other harnesses often break YAML, such as with a colon
//! in a plain value, and Mannie needs only a few top-level scalars. So this
//! reader takes the top-level keys with plain, quoted, literal and folded
//! values, skips nested maps and lists, ignores comments and expands nothing:
//! an alias is no value, so a hostile file cannot make the reader produce more
//! text than it holds.

/// The values of a frontmatter's top-level keys, and where the body starts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Frontmatter {
    /// Each top-level key with a scalar value, in the order of the text. A
    /// repeated key keeps its first value.
    values: Vec<(String, String)>,
    /// Where the body starts, in bytes of the text that was read.
    body_start: usize,
}

impl Frontmatter {
    /// The value of a top-level key, or `None` when the key is missing or
    /// holds no scalar.
    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(candidate, _value)| candidate == key)
            .map(|(_key, value)| value.as_str())
    }

    /// The text after the frontmatter's closing line.
    pub(crate) fn body<'a>(&self, text: &'a str) -> &'a str {
        text.get(self.body_start..).unwrap_or_default()
    }
}

/// Why a text has no frontmatter that Mannie reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrontmatterError {
    /// The text does not start with a line `---`.
    Missing,
    /// The frontmatter has no closing line.
    NotClosed,
}

/// One line of the text, with where it starts and its text without its line
/// break.
#[derive(Debug, Clone, Copy)]
struct Line<'a> {
    text: &'a str,
    /// The byte where the next line starts.
    next: usize,
}

/// The lines of a text, each without its LF or CRLF.
fn split_lines(text: &str, start: usize) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    let mut position = start;
    while position < text.len() {
        let rest = &text[position..];
        let (line, length) = match rest.find('\n') {
            Some(end) => (&rest[..end], end + 1),
            None => (rest, rest.len()),
        };
        position += length;
        lines.push(Line {
            text: line.strip_suffix('\r').unwrap_or(line),
            next: position,
        });
    }
    lines
}

/// Reads the frontmatter at the start of a text.
pub(crate) fn read_frontmatter(text: &str) -> Result<Frontmatter, FrontmatterError> {
    let start = if text.starts_with('\u{FEFF}') {
        '\u{FEFF}'.len_utf8()
    } else {
        0
    };
    let lines = split_lines(text, start);
    let opens = lines
        .first()
        .is_some_and(|line| line.text.trim_end() == "---");
    if !opens {
        return Err(FrontmatterError::Missing);
    }
    let closing = lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_index, line)| matches!(line.text.trim_end(), "---" | "..."))
        .map(|(index, _line)| index)
        .ok_or(FrontmatterError::NotClosed)?;
    let body_start = lines[closing].next;
    let texts: Vec<&str> = lines[1..closing].iter().map(|line| line.text).collect();
    Ok(Frontmatter {
        values: top_level_values(&texts),
        body_start,
    })
}

/// The number of spaces before a line's text. A tab counts as one, which is
/// enough to tell top-level lines from indented ones.
fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

/// Whether a line holds nothing but white space or a comment.
fn is_blank(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.is_empty() || trimmed.starts_with('#')
}

/// The key of a top-level line, `key:` followed by a space, a tab or the end
/// of the line, and the text after the colon. A line that starts with white
/// space, a list's `-`, a quote or a flow collection holds no plain key.
fn key_of(line: &str) -> Option<(&str, &str)> {
    if indent(line) > 0 || line.starts_with(['-', '"', '\'', '[', '{', '?', '#']) {
        return None;
    }
    let mut search = 0;
    while let Some(found) = line[search..].find(':') {
        let colon = search + found;
        let after = &line[colon + 1..];
        if after.is_empty() || after.starts_with([' ', '\t']) {
            let key = line[..colon].trim_end();
            return (!key.is_empty()).then_some((key, after));
        }
        search = colon + 1;
    }
    None
}

/// The top-level keys with a scalar value. Indented lines that follow a key
/// belong to its value, or to a nested map or list, which is skipped.
fn top_level_values(lines: &[&str]) -> Vec<(String, String)> {
    let mut values: Vec<(String, String)> = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        index += 1;
        let Some((key, after)) = key_of(line) else {
            continue;
        };
        // The value's lines: every following line that is blank or indented.
        let mut end = index;
        while end < lines.len() && (lines[end].trim().is_empty() || indent(lines[end]) > 0) {
            end += 1;
        }
        let value = scalar(after, &lines[index..end]);
        index = end;
        let repeated = values.iter().any(|(existing, _value)| existing == key);
        if let (Some(value), false) = (value, repeated) {
            values.push((key.to_owned(), value));
        }
    }
    values
}

/// The scalar after a key's colon, with the indented lines that follow it, or
/// `None` for a nested map or list, a flow collection or an alias.
fn scalar(after: &str, following: &[&str]) -> Option<String> {
    let mut first = after.trim();
    // A tag or an anchor only names the value, which follows it.
    while first.starts_with(['!', '&']) {
        first = first
            .split_once([' ', '\t'])
            .map_or("", |(_name, rest)| rest.trim_start());
    }
    match first.chars().next() {
        None | Some('#') => plain_on_next_lines(following),
        // An alias is never expanded.
        Some('*' | '[' | '{') => None,
        Some('|' | '>') => Some(block(first, following)),
        Some(quote @ ('"' | '\'')) => Some(quoted(quote, first, following)),
        Some(_) => Some(plain(first, following)),
    }
}

/// Whether an indented line starts a nested map or list.
fn starts_nested(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed == "-" || trimmed.starts_with("- ") || key_of(trimmed).is_some()
}

/// A value that starts on the line after its key: a plain scalar on indented
/// lines, or `None` for a nested map or list.
fn plain_on_next_lines(following: &[&str]) -> Option<String> {
    let first = following.iter().find(|line| !is_blank(line))?;
    if starts_nested(first) {
        return None;
    }
    let value = fold(following.iter().map(|line| strip_comment(line.trim())));
    (!value.is_empty()).then_some(value)
}

/// A plain value: the rest of its line without a comment, folded with the
/// indented lines that continue it. A colon inside it is kept, as other
/// clients that read skills tolerate it.
fn plain(first: &str, following: &[&str]) -> String {
    let lines = std::iter::once(strip_comment(first))
        .chain(following.iter().map(|line| strip_comment(line.trim())));
    fold(lines)
}

/// A line without a comment: `#` after white space starts one.
fn strip_comment(line: &str) -> &str {
    let mut previous_is_space = true;
    for (position, character) in line.char_indices() {
        if character == '#' && previous_is_space {
            return line[..position].trim_end();
        }
        previous_is_space = character == ' ' || character == '\t';
    }
    line.trim_end()
}

/// Lines folded as YAML folds flow scalars: lines join with a space, and an
/// empty line becomes a line break.
fn fold<'a>(lines: impl Iterator<Item = &'a str>) -> String {
    let mut folded = String::new();
    let mut breaks = 0_usize;
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            breaks += 1;
            continue;
        }
        if !folded.is_empty() {
            if breaks == 0 {
                folded.push(' ');
            } else {
                folded.push_str(&"\n".repeat(breaks));
            }
        }
        breaks = 0;
        folded.push_str(line);
    }
    folded
}

/// A literal (`|`) or folded (`>`) block, whose lines are the indented lines
/// that follow, with the chomping that its header names.
fn block(header: &str, following: &[&str]) -> String {
    let header = strip_comment(header);
    let literal = header.starts_with('|');
    let strip = header.contains('-');
    let keep = header.contains('+');
    let explicit = header
        .chars()
        .find_map(|character| character.to_digit(10))
        .and_then(|digit| usize::try_from(digit).ok());
    let content_indent = explicit.unwrap_or_else(|| {
        following
            .iter()
            .find(|line| !line.trim().is_empty())
            .map_or(0, |line| indent(line))
    });
    let lines: Vec<&str> = following
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                ""
            } else {
                line.get(content_indent..)
                    .unwrap_or_else(|| line.trim_start())
            }
        })
        .collect();
    let mut text = if literal {
        lines.join("\n")
    } else {
        fold_block(&lines)
    };
    let content_end = text.trim_end_matches('\n').len();
    if keep {
        text.push('\n');
    } else {
        text.truncate(content_end);
        if !strip && !text.is_empty() {
            text.push('\n');
        }
    }
    text
}

/// The lines of a folded block: lines join with a space, each empty line
/// becomes a line break in place of that space, and lines indented further
/// keep their breaks.
fn fold_block(lines: &[&str]) -> String {
    let mut folded = String::new();
    let mut empty_lines = 0_usize;
    let mut started = false;
    let mut previous_more_indented = false;
    for line in lines {
        if line.is_empty() {
            empty_lines += 1;
            continue;
        }
        let more_indented = line.starts_with([' ', '\t']);
        if !started || empty_lines > 0 {
            folded.push_str(&"\n".repeat(empty_lines));
        } else if more_indented || previous_more_indented {
            folded.push('\n');
        } else {
            folded.push(' ');
        }
        folded.push_str(line);
        started = true;
        empty_lines = 0;
        previous_more_indented = more_indented;
    }
    folded.push_str(&"\n".repeat(empty_lines));
    folded
}

/// A quoted value, which may continue on the following lines up to its
/// closing quote. One that never closes is read as a plain value.
fn quoted(quote: char, first: &str, following: &[&str]) -> String {
    let mut lines = vec![&first[quote.len_utf8()..]];
    lines.extend(following.iter().map(|line| line.trim()));
    let mut parts: Vec<String> = Vec::new();
    for line in &lines {
        match closing_quote(quote, line) {
            Some(end) => {
                parts.push(line[..end].to_owned());
                let joined = fold(parts.iter().map(String::as_str));
                return if quote == '"' {
                    unescape(&joined)
                } else {
                    joined.replace("''", "'")
                };
            }
            None => parts.push((*line).to_owned()),
        }
    }
    plain(first, following)
}

/// Where the closing quote of a quoted value's line is, if the line holds it.
fn closing_quote(quote: char, line: &str) -> Option<usize> {
    let mut characters = line.char_indices().peekable();
    while let Some((position, character)) = characters.next() {
        if quote == '"' && character == '\\' {
            characters.next();
            continue;
        }
        if character == quote {
            // In single quotes, `''` is a quote inside the value.
            if quote == '\'' && characters.peek().is_some_and(|(_next, next)| *next == '\'') {
                characters.next();
                continue;
            }
            return Some(position);
        }
    }
    None
}

/// The text of a double-quoted value with its escapes resolved. An escape
/// that YAML does not define keeps its backslash.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        let Some(escaped) = characters.next() else {
            out.push('\\');
            break;
        };
        match escaped {
            'n' => out.push('\n'),
            't' | '\t' => out.push('\t'),
            'r' => out.push('\r'),
            '0' => out.push('\0'),
            'a' => out.push('\u{7}'),
            'b' => out.push('\u{8}'),
            'e' => out.push('\u{1b}'),
            'f' => out.push('\u{c}'),
            'v' => out.push('\u{b}'),
            'N' => out.push('\u{85}'),
            '_' => out.push('\u{a0}'),
            'L' => out.push('\u{2028}'),
            'P' => out.push('\u{2029}'),
            ' ' | '"' | '/' | '\\' => out.push(escaped),
            'x' | 'u' | 'U' => {
                let digits = match escaped {
                    'x' => 2,
                    'u' => 4,
                    _ => 8,
                };
                let hex: String = characters.by_ref().take(digits).collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(decoded) if hex.len() == digits => out.push(decoded),
                    _ => {
                        out.push('\\');
                        out.push(escaped);
                        out.push_str(&hex);
                    }
                }
            }
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

/// A YAML boolean as skills write it: `true` or `false` in any case.
pub(crate) fn flag(value: &str) -> Option<bool> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("true") {
        Some(true)
    } else if value.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{FrontmatterError, flag, read_frontmatter};

    fn values(text: &str) -> Vec<(String, String)> {
        read_frontmatter(text).unwrap().values
    }

    fn value(text: &str, key: &str) -> Option<String> {
        read_frontmatter(text).unwrap().get(key).map(str::to_owned)
    }

    /// The start of this repository's skill, `.agents/skills/shadcn-svelte/
    /// SKILL.md`, as its authors wrote it: a description with a dash, quotes
    /// and commas, and two fields of Claude Code.
    const SHADCN_SVELTE: &str = r#"---
name: shadcn-svelte
description: Manages shadcn-svelte components and projects — adding, updating, fixing, debugging, styling, and composing UI. Provides project context, component docs, and usage examples. Applies when working with shadcn-svelte, the CLI, design-system presets, or any project with a components.json file. Also triggers for "shadcn-svelte init", "add component", or registry URLs.
user-invocable: false
allowed-tools: Bash(npx shadcn-svelte@latest *), Bash(pnpm dlx shadcn-svelte@latest *), Bash(bunx --bun shadcn-svelte@latest *)
---

# shadcn-svelte
"#;

    #[test]
    fn this_repository_s_skill_reads_with_its_fields_and_body() {
        let text = SHADCN_SVELTE;

        let frontmatter = read_frontmatter(text).unwrap();

        assert_eq!(frontmatter.get("name"), Some("shadcn-svelte"));
        let description = frontmatter.get("description").unwrap();
        assert!(
            description.starts_with("Manages shadcn-svelte components and projects"),
            "{description}"
        );
        assert!(description.contains("\"add component\""), "{description}");
        assert_eq!(frontmatter.get("user-invocable"), Some("false"));
        assert!(frontmatter.get("allowed-tools").is_some());
        assert_eq!(frontmatter.body(text), "\n# shadcn-svelte\n");
    }

    #[test]
    fn plain_values_keep_colons_and_drop_comments() {
        let text =
            "---\nname: pdf # the name\ndescription: Use when: forms, tables. C# too\n---\nBody\n";

        assert_eq!(value(text, "name").as_deref(), Some("pdf"));
        assert_eq!(
            value(text, "description").as_deref(),
            Some("Use when: forms, tables. C# too")
        );
    }

    #[test]
    fn a_plain_value_continues_on_indented_lines() {
        let text = "---\ndescription: Fills forms\n  and reads tables.\n\n  Second paragraph.\nname: pdf\n---\n";

        assert_eq!(
            value(text, "description").as_deref(),
            Some("Fills forms and reads tables.\nSecond paragraph.")
        );
        assert_eq!(value(text, "name").as_deref(), Some("pdf"));
    }

    #[test]
    fn quoted_values_resolve_their_escapes_and_may_span_lines() {
        let text = concat!(
            "---\n",
            "double: \"Say \\\"hi\\\"\\tnow \\u00e9\\x41\"\n",
            "single: 'It''s: fine'\n",
            "long: \"First line\n  second line\"\n",
            "open: \"never closes\n",
            "---\n"
        );

        assert_eq!(value(text, "double").as_deref(), Some("Say \"hi\"\tnow éA"));
        assert_eq!(value(text, "single").as_deref(), Some("It's: fine"));
        assert_eq!(
            value(text, "long").as_deref(),
            Some("First line second line")
        );
        assert_eq!(value(text, "open").as_deref(), Some("\"never closes"));
    }

    #[test]
    fn literal_and_folded_blocks_follow_their_chomping() {
        let text = concat!(
            "---\n",
            "literal: |\n  one\n  two\n\n",
            "folded: >-\n  one\n  two\n\n  three\n",
            "kept: |+\n  line\n",
            "---\n"
        );

        assert_eq!(value(text, "literal").as_deref(), Some("one\ntwo\n"));
        assert_eq!(value(text, "folded").as_deref(), Some("one two\nthree"));
        assert_eq!(value(text, "kept").as_deref(), Some("line\n"));
    }

    #[test]
    fn nested_maps_lists_flow_collections_and_aliases_hold_no_scalar() {
        let text = concat!(
            "---\n",
            "metadata:\n  author: someone\n  version: \"1.0\"\n",
            "tags:\n  - one\n  - two\n",
            "flow: [a, b]\n",
            "base: &anchor The anchored text\n",
            "copy: *anchor\n",
            "typed: !!str Typed text\n",
            "name: pdf\n",
            "---\n"
        );

        assert_eq!(
            values(text),
            [
                ("base".to_owned(), "The anchored text".to_owned()),
                ("typed".to_owned(), "Typed text".to_owned()),
                ("name".to_owned(), "pdf".to_owned()),
            ]
        );
    }

    #[test]
    fn a_repeated_key_keeps_its_first_value() {
        assert_eq!(
            value("---\nname: first\nname: second\n---\n", "name").as_deref(),
            Some("first")
        );
    }

    #[test]
    fn a_byte_order_mark_crlf_and_the_closing_dots_are_read() {
        let text = "\u{FEFF}---\r\nname: pdf\r\ndescription: Forms\r\n...\r\nBody\r\n";

        let frontmatter = read_frontmatter(text).unwrap();

        assert_eq!(frontmatter.get("name"), Some("pdf"));
        assert_eq!(frontmatter.get("description"), Some("Forms"));
        assert_eq!(frontmatter.body(text), "Body\r\n");
    }

    #[test]
    fn a_text_without_an_opening_or_a_closing_line_has_no_frontmatter() {
        assert_eq!(
            read_frontmatter("# Title\n---\nname: x\n---\n"),
            Err(FrontmatterError::Missing)
        );
        assert_eq!(read_frontmatter(""), Err(FrontmatterError::Missing));
        assert_eq!(
            read_frontmatter("---\nname: x\ndescription: never closed\n"),
            Err(FrontmatterError::NotClosed)
        );
    }

    #[test]
    fn a_key_needs_a_space_after_its_colon() {
        let text = "---\nhttp://example.com\nname: pdf\nurl:https\n---\n";

        assert_eq!(values(text), [("name".to_owned(), "pdf".to_owned())]);
    }

    #[test]
    fn flags_read_true_and_false_in_any_case() {
        assert_eq!(flag(" TRUE "), Some(true));
        assert_eq!(flag("False"), Some(false));
        assert_eq!(flag("yes"), None);
        assert_eq!(flag(""), None);
    }
}
