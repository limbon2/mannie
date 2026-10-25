//! The passages of a document: pieces of about the target length that end at
//! a paragraph, then a line, then a sentence, then a word, and share some text
//! with the next piece, never across sections. Lengths count characters, so
//! no tokenizer of a model is involved; a model's server reads them in its
//! own tokens.

use crate::Document;

/// The version of the passages that [`ChunkOptions::default`] makes. The
/// plugin splits a source's text again when its passages are of an earlier
/// version, so raise it whenever the passages change.
pub const CHUNKER_VERSION: u32 = 2;

/// How long passages are and how much text the next one repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkOptions {
    /// The characters that a passage aims for.
    pub target: usize,
    /// The characters at the end of a passage that the next one begins with,
    /// about.
    pub overlap: usize,
}

impl Default for ChunkOptions {
    /// Version 2: the sizes of the earlier harness, whose answers the owner
    /// found better (ADR-0049, context), which version 1's 1,200 and 150 were
    /// smaller than.
    fn default() -> Self {
        Self {
            target: 1_500,
            overlap: 200,
        }
    }
}

/// A passage: a piece of one section's text, by the byte offsets of its start
/// and end in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Passage {
    /// The index of its section in the document.
    pub section: usize,
    /// The byte offset of its first character in the section's text.
    pub start: usize,
    /// The byte offset after its last character.
    pub end: usize,
}

/// The passages of every section of a document, in order.
pub fn passages(document: &Document, options: ChunkOptions) -> Vec<Passage> {
    let target = options.target.max(1);
    let overlap = options.overlap.min(target / 2);
    let mut found = Vec::new();
    for (section, content) in document.sections.iter().enumerate() {
        for (start, end) in split(&content.text, target, overlap) {
            found.push(Passage {
                section,
                start,
                end,
            });
        }
    }
    found
}

/// The byte ranges of the passages of one text.
fn split(text: &str, target: usize, overlap: usize) -> Vec<(usize, usize)> {
    // The byte offset of each character, and the text's end.
    let offsets: Vec<usize> = text
        .char_indices()
        .map(|(offset, _character)| offset)
        .chain(std::iter::once(text.len()))
        .collect();
    let characters = offsets.len() - 1;
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < characters {
        let remaining = characters - start;
        // A tail shorter than a fifth of a passage joins the passage before
        // it, so that no passage is a few words.
        let end = if remaining <= target + target / 5 {
            characters
        } else {
            boundary(text, &offsets, start, start + target)
        };
        let (trimmed_start, trimmed_end) = trim(text, &offsets, start, end);
        if trimmed_start < trimmed_end {
            ranges.push((offsets[trimmed_start], offsets[trimmed_end]));
        }
        if end >= characters {
            break;
        }
        start = next_start(text, &offsets, start, end, overlap);
    }
    ranges
}

/// The best end of a passage at or before `limit`, in characters: after a
/// paragraph's end, then a line's end, then a sentence's end, then a space,
/// in the last half of the passage, or else `limit` itself.
fn boundary(text: &str, offsets: &[usize], start: usize, limit: usize) -> usize {
    let earliest = start + (limit - start) / 2;
    let window = &text[offsets[earliest]..offsets[limit]];
    for find in BREAKS {
        if let Some(after) = find(window) {
            let byte = offsets[earliest] + after;
            if let Ok(character) = offsets.binary_search(&byte)
                && character > start
            {
                return character;
            }
        }
    }
    limit
}

/// The places where a passage may end, the best first. Each gives the byte
/// offset after the last such place in a window.
const BREAKS: [fn(&str) -> Option<usize>; 4] =
    [after_paragraph, after_line, after_sentence, after_space];

/// The marks that end a sentence.
const SENTENCE_ENDS: [char; 6] = ['.', '!', '?', '。', '！', '？'];

fn after_paragraph(window: &str) -> Option<usize> {
    window.rfind("\n\n").map(|at| at + 2)
}

fn after_line(window: &str) -> Option<usize> {
    window.rfind('\n').map(|at| at + 1)
}

fn after_sentence(window: &str) -> Option<usize> {
    window
        .char_indices()
        .rev()
        .filter(|(_at, mark)| SENTENCE_ENDS.contains(mark))
        .map(|(at, mark)| (at + mark.len_utf8(), mark))
        .find(|&(after, mark)| {
            // A full stop of Chinese or Japanese text has no space after it,
            // and a period in "3.5" or "e.g." is no sentence's end.
            !mark.is_ascii()
                || window[after..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace)
        })
        .map(|(after, _mark)| after)
}

fn after_space(window: &str) -> Option<usize> {
    window
        .char_indices()
        .rev()
        .find(|(_at, character)| character.is_whitespace())
        .map(|(at, character)| at + character.len_utf8())
}

/// Where the next passage starts: `overlap` characters before the end, moved
/// on to the start of a word, and always after the previous start.
fn next_start(text: &str, offsets: &[usize], start: usize, end: usize, overlap: usize) -> usize {
    let mut next = end.saturating_sub(overlap).max(start + 1);
    while next < end {
        let previous_is_space = text[offsets[next - 1]..offsets[next]]
            .chars()
            .all(char::is_whitespace);
        if previous_is_space {
            break;
        }
        next += 1;
    }
    next
}

/// The range without white space at its ends, in characters.
fn trim(text: &str, offsets: &[usize], start: usize, end: usize) -> (usize, usize) {
    let is_space = |character: usize| {
        text[offsets[character]..offsets[character + 1]]
            .chars()
            .all(char::is_whitespace)
    };
    let mut first = start;
    while first < end && is_space(first) {
        first += 1;
    }
    let mut last = end;
    while last > first && is_space(last - 1) {
        last -= 1;
    }
    (first, last)
}

#[cfg(test)]
mod tests {
    use super::{ChunkOptions, passages};
    use crate::{Document, Location, Section};

    fn document(texts: &[&str]) -> Document {
        Document {
            sections: texts
                .iter()
                .enumerate()
                .map(|(page, text)| Section {
                    location: Location::Page(u32::try_from(page + 1).unwrap()),
                    text: (*text).to_owned(),
                })
                .collect(),
            pages: None,
        }
    }

    fn pieces(document: &Document, options: ChunkOptions) -> Vec<(usize, String)> {
        passages(document, options)
            .into_iter()
            .map(|passage| {
                let text = &document.sections[passage.section].text[passage.start..passage.end];
                (passage.section, text.to_owned())
            })
            .collect()
    }

    #[test]
    fn a_short_section_is_one_passage_and_sections_never_share_one() {
        let found = pieces(
            &document(&["A short page.", "Another page."]),
            ChunkOptions::default(),
        );

        assert_eq!(
            found,
            [
                (0, "A short page.".to_owned()),
                (1, "Another page.".to_owned())
            ]
        );
    }

    #[test]
    fn passages_end_at_paragraphs_before_sentences_and_words() {
        let text = format!(
            "{}\n\n{}",
            "First paragraph. ".repeat(4).trim(),
            "Second one here. ".repeat(4).trim()
        );
        let options = ChunkOptions {
            target: 80,
            overlap: 0,
        };

        let found = pieces(&document(&[&text]), options);

        assert_eq!(found[0].1, "First paragraph. ".repeat(4).trim());
        assert!(found[1].1.starts_with("Second one here."));
    }

    #[test]
    fn a_passage_without_breaks_ends_at_a_sentence_then_at_a_space() {
        let sentences = "One two three. Four five six. Seven eight nine. Ten eleven twelve.";
        let words = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu";
        let options = ChunkOptions {
            target: 40,
            overlap: 0,
        };

        let by_sentence = pieces(&document(&[sentences]), options);
        let by_word = pieces(&document(&[words]), options);

        assert!(by_sentence[0].1.ends_with('.'), "{by_sentence:?}");
        assert!(!by_word[0].1.ends_with(' '));
        assert!(words.starts_with(&by_word[0].1));
        assert!(by_word.iter().all(|(_section, text)| !text.contains("  ")));
    }

    #[test]
    fn passages_overlap_and_start_at_a_word() {
        let text = "word ".repeat(100);
        let options = ChunkOptions {
            target: 100,
            overlap: 30,
        };

        let found = passages(&document(&[text.trim()]), options);

        for pair in found.windows(2) {
            assert!(pair[1].start < pair[0].end, "{pair:?}");
            assert!(text[pair[1].start..].starts_with("word"));
        }
    }

    #[test]
    fn every_character_is_in_a_passage_and_passages_cut_text_at_character_boundaries() {
        let text = "Grüße aus Köln. 日本語の文章です。".repeat(40);
        let options = ChunkOptions {
            target: 50,
            overlap: 10,
        };
        let doc = document(&[&text]);

        let found = passages(&doc, options);

        assert!(found.len() > 5);
        assert_eq!(found.first().map(|passage| passage.start), Some(0));
        assert_eq!(found.last().map(|passage| passage.end), Some(text.len()));
        for pair in found.windows(2) {
            // Only white space, which passages trim, may fall between two.
            if pair[1].start > pair[0].end {
                let gap = &text[pair[0].end..pair[1].start];
                assert!(
                    gap.chars().all(char::is_whitespace),
                    "text between {pair:?}"
                );
            }
        }
        for passage in found {
            assert!(text.is_char_boundary(passage.start) && text.is_char_boundary(passage.end));
        }
    }

    #[test]
    fn a_short_tail_joins_the_passage_before_it() {
        let text = format!("{} tail", "x".repeat(100));
        let options = ChunkOptions {
            target: 100,
            overlap: 0,
        };

        let found = pieces(&document(&[&text]), options);

        assert_eq!(found.len(), 1);
        assert!(found[0].1.ends_with("tail"));
    }
}
