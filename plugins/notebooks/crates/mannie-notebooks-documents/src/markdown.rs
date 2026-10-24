//! The sections of a Markdown file (plugin ADR-0001, decision 3): a scanner
//! of its lines that finds ATX and setext headings outside fenced code, and
//! names each section by the path of its headings. Front matter between
//! `---` lines at the top is left out. Passages need only this structure, so
//! no Markdown parser is involved.

use crate::{Document, Location, Section, tidy};

/// The deepest heading, as Markdown has it.
const MAX_LEVEL: usize = 6;

/// A heading's level and title.
struct Heading {
    level: usize,
    title: String,
}

/// An open fence of code: its character and how many of them opened it.
#[derive(Clone, Copy)]
struct Fence {
    marker: char,
    length: usize,
}

/// The sections of a Markdown text, in order.
pub(crate) fn extract(text: &str) -> Document {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = without_front_matter(&text).split('\n').collect();
    let mut sections = Vec::new();
    let mut path: Vec<(usize, String)> = Vec::new();
    let mut body: Vec<&str> = Vec::new();
    let mut fence: Option<Fence> = None;
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if let Some(open) = fence {
            if closes(line, open) {
                fence = None;
            }
            body.push(line);
            index += 1;
            continue;
        }
        if let Some(open) = opening_fence(line) {
            fence = Some(open);
            body.push(line);
            index += 1;
            continue;
        }
        let setext = lines
            .get(index + 1)
            .and_then(|next| setext_level(line, next));
        let heading = atx_heading(line).or_else(|| {
            setext.map(|level| Heading {
                level,
                title: line.trim().to_owned(),
            })
        });
        let Some(heading) = heading else {
            body.push(line);
            index += 1;
            continue;
        };
        push_section(&mut sections, &path, &body);
        body.clear();
        path.retain(|(level, _title)| *level < heading.level);
        path.push((heading.level, heading.title));
        // A setext heading's underline belongs to it. A line that is an ATX
        // heading is never a setext title, so `setext` is only its own.
        index += if setext.is_some() { 2 } else { 1 };
    }
    push_section(&mut sections, &path, &body);
    Document {
        sections,
        pages: None,
    }
}

/// The text after front matter: a first line of `---`, up to a line of `---`
/// or `...`. Without a closing line, nothing is front matter.
fn without_front_matter(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("---\n") else {
        return text;
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        let bare = line.trim_end();
        if bare == "---" || bare == "..." {
            return &rest[offset + line.len()..];
        }
        offset += line.len();
    }
    text
}

fn push_section(sections: &mut Vec<Section>, path: &[(usize, String)], body: &[&str]) {
    let text = tidy(&body.join("\n"));
    if text.is_empty() {
        return;
    }
    let headings = path.iter().map(|(_level, title)| title.clone()).collect();
    sections.push(Section {
        location: Location::Headings(headings),
        text,
    });
}

/// The indentation of a line in spaces, when it is at most three, as
/// Markdown allows before a heading or a fence.
fn indentation(line: &str) -> Option<usize> {
    let spaces = line.len() - line.trim_start_matches(' ').len();
    (spaces <= 3).then_some(spaces)
}

/// An ATX heading: one to six `#`, then white space or the end of the line.
/// The closing `#` go, and so do they when they are the whole title.
fn atx_heading(line: &str) -> Option<Heading> {
    let spaces = indentation(line)?;
    let rest = &line[spaces..];
    let level = rest.len() - rest.trim_start_matches('#').len();
    if level == 0 || level > MAX_LEVEL {
        return None;
    }
    let after = &rest[level..];
    if !after.is_empty() && !after.starts_with([' ', '\t']) {
        return None;
    }
    let mut title = after.trim();
    let without_closing = title.trim_end_matches('#');
    if without_closing.is_empty() || without_closing.ends_with([' ', '\t']) {
        title = without_closing.trim_end();
    }
    Some(Heading {
        level,
        title: title.to_owned(),
    })
}

/// The level of a setext heading whose title is `line` and whose underline
/// is `next`: `=` gives 1 and `-` gives 2. The title is a line of text, not
/// blank, not a heading, a list item, a quote or a fence of its own.
fn setext_level(line: &str, next: &str) -> Option<usize> {
    let is_text = indentation(line).is_some()
        && !line.trim().is_empty()
        && atx_heading(line).is_none()
        && opening_fence(line).is_none()
        && !line.trim_start().starts_with(['>', '-', '*', '+', '|']);
    if !is_text {
        return None;
    }
    let spaces = indentation(next)?;
    let underline = next[spaces..].trim_end();
    if underline.is_empty() {
        return None;
    }
    if underline.chars().all(|character| character == '=') {
        Some(1)
    } else if underline.chars().all(|character| character == '-') {
        Some(2)
    } else {
        None
    }
}

/// A fence that opens code: three or more backticks or tildes. A backtick
/// fence's info string holds no backtick.
fn opening_fence(line: &str) -> Option<Fence> {
    let spaces = indentation(line)?;
    let rest = &line[spaces..];
    let marker = rest.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let length = rest.len() - rest.trim_start_matches(marker).len();
    if length < 3 {
        return None;
    }
    if marker == '`' && rest[length..].contains('`') {
        return None;
    }
    Some(Fence { marker, length })
}

/// Whether a line closes the fence: at least as many of its character, and
/// nothing but white space after them.
fn closes(line: &str, open: Fence) -> bool {
    let Some(spaces) = indentation(line) else {
        return false;
    };
    let rest = line[spaces..].trim_end();
    let length = rest.len() - rest.trim_start_matches(open.marker).len();
    length >= open.length && length == rest.len()
}

#[cfg(test)]
mod tests {
    use super::extract;
    use crate::Location;

    fn headings(path: &[&str]) -> Location {
        Location::Headings(path.iter().map(|&title| title.to_owned()).collect())
    }

    fn sections(text: &str) -> Vec<(Location, String)> {
        extract(text)
            .sections
            .into_iter()
            .map(|section| (section.location, section.text))
            .collect()
    }

    #[test]
    fn headings_name_the_sections_by_their_path() {
        let text = "Intro text.\n\n# Physics\n\nAbout physics.\n\n## Kinematics\n\nMotion.\n\n### Projectiles\n\nArcs.\n\n## Dynamics ##\n\nForces.\n\n# Chemistry\n\nAtoms.";

        assert_eq!(
            sections(text),
            [
                (headings(&[]), "Intro text.".to_owned()),
                (headings(&["Physics"]), "About physics.".to_owned()),
                (headings(&["Physics", "Kinematics"]), "Motion.".to_owned()),
                (
                    headings(&["Physics", "Kinematics", "Projectiles"]),
                    "Arcs.".to_owned()
                ),
                (headings(&["Physics", "Dynamics"]), "Forces.".to_owned()),
                (headings(&["Chemistry"]), "Atoms.".to_owned()),
            ]
        );
    }

    #[test]
    fn lines_in_fenced_code_are_never_headings() {
        let text = "# Setup\n\n```sh\n# install it\nmake\n```\n\n~~~~\n## also code\n~~~\nstill code\n~~~~\n\nAfter.";

        let found = sections(text);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, headings(&["Setup"]));
        assert!(found[0].1.contains("# install it"));
        assert!(found[0].1.contains("## also code"));
        assert!(found[0].1.ends_with("After."));
    }

    #[test]
    fn setext_headings_take_their_underline() {
        let text = "Title\n=====\n\nBody.\n\nPart\n----\n\nMore.";

        assert_eq!(
            sections(text),
            [
                (headings(&["Title"]), "Body.".to_owned()),
                (headings(&["Title", "Part"]), "More.".to_owned()),
            ]
        );
    }

    #[test]
    fn front_matter_is_left_out_and_text_that_only_looks_like_a_heading_stays() {
        let text = "---\ntitle: Notes\ntags: [a]\n---\n#hashtag is no heading\n####### seven is text\n- item\n---\n\nEnd.";

        let found = sections(text);

        assert_eq!(found.len(), 1);
        assert!(!found[0].1.contains("title: Notes"));
        assert!(
            found[0]
                .1
                .starts_with("#hashtag is no heading\n####### seven is text")
        );
    }

    #[test]
    fn front_matter_without_a_closing_line_is_text() {
        let found = sections("---\nnot closed\n\nBody.");

        assert!(found[0].1.contains("not closed"));
    }

    #[test]
    fn empty_sections_are_left_out_and_windows_line_endings_read_as_lines() {
        let text = "# Empty\r\n\r\n# Full\r\nText.\r\n";

        assert_eq!(sections(text), [(headings(&["Full"]), "Text.".to_owned())]);
    }
}
