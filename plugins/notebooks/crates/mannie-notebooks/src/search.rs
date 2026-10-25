//! The search of a notebook's passages (plugins/notebooks/docs/README.md,
//! search): by meaning, the cosine of the query's vector and each passage's;
//! by words, BM25 over the passages' words; the two fused by reciprocal rank;
//! and the choice of the passages that go, best first, without two that
//! share most of their text.
//!
//! A corpus holds one notebook's passages in memory with their words, which
//! the plugin keeps after a search until the notebook changes or the memory
//! is needed for another (`library.rs`).

use std::collections::HashMap;
use std::hash::Hash;

use crate::store::CorpusRow;

/// How many passages each ranking keeps for the fusion.
pub(crate) const CANDIDATES: usize = 50;

/// How many of the fused passages the reranker scores, as many as the
/// earlier harness had it score (ADR-0049, context).
pub(crate) const RERANKED: usize = 40;

/// The constant of reciprocal rank fusion.
const FUSION_K: f32 = 60.0;

/// BM25's saturation of a word's count.
const BM25_K1: f32 = 1.2;

/// BM25's weight of a passage's length.
const BM25_B: f32 = 0.75;

/// The most characters of the owner's message that a search reads.
pub(crate) const MAX_QUERY_CHARACTERS: usize = 4_000;

/// Common English words, which match too many passages to rank them, as the
/// earlier harness leaves them out.
const STOP_WORDS: [&str; 61] = [
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "can", "do", "does", "for", "from",
    "has", "have", "how", "i", "if", "in", "into", "is", "it", "its", "no", "not", "of", "on",
    "or", "such", "that", "the", "their", "then", "there", "these", "they", "this", "to", "was",
    "we", "were", "what", "when", "where", "which", "who", "why", "will", "with", "you", "your",
    "our", "us", "my", "me", "so", "than", "too", "very", "also",
];

/// The longest word that a search keeps, in characters.
const MAX_WORD_CHARACTERS: usize = 64;

/// Whether a character belongs to a script written without spaces between
/// words: Han, Hiragana, Katakana and Hangul.
fn without_spaces(character: char) -> bool {
    matches!(
        u32::from(character),
        0x1100..=0x11FF
            | 0x3040..=0x30FF
            | 0x3130..=0x318F
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xAC00..=0xD7AF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2FA1F
    )
}

/// A word as a search keeps it: in lower case, and with an English plural's
/// ending folded, so that "policies" finds "policy" and "hosts" finds "host".
fn stem(word: &str) -> String {
    let lower = word.to_lowercase();
    let characters = lower.chars().count();
    if characters > 4
        && let Some(root) = lower.strip_suffix("ies")
    {
        return format!("{root}y");
    }
    let is_plural = characters > 3 && lower.ends_with('s') && !lower.ends_with("ss");
    if is_plural {
        return lower[..lower.len() - 1].to_owned();
    }
    lower
}

/// The parts of an identifier: split at underscores, after a lower-case
/// letter or a digit before an upper-case one, and before the last capital
/// of a run of them, so that `HTTPServer_port` gives HTTP, Server and port.
fn identifier_parts(word: &str) -> Vec<&str> {
    let characters: Vec<(usize, char)> = word.char_indices().collect();
    let mut parts = Vec::new();
    let mut start = 0;
    for (index, &(at, character)) in characters.iter().enumerate() {
        if character == '_' {
            parts.push(&word[start..at]);
            start = at + character.len_utf8();
            continue;
        }
        let Some(&(_before_at, before)) = index.checked_sub(1).and_then(|i| characters.get(i))
        else {
            continue;
        };
        let after = characters.get(index + 1).map(|&(_at, after)| after);
        let rises = (before.is_lowercase() || before.is_numeric()) && character.is_uppercase();
        let ends_capitals = before.is_uppercase()
            && character.is_uppercase()
            && after.is_some_and(char::is_lowercase);
        if rises || ends_capitals {
            parts.push(&word[start..at]);
            start = at;
        }
    }
    parts.push(&word[start..]);
    parts
        .into_iter()
        .filter(|part| part.chars().count() > 1)
        .collect()
}

/// Adds a word, unless it is a stop word or too long. A stop word is known
/// before its ending is folded, so "does" goes and "hosts" stays.
fn add_word(found: &mut Vec<String>, word: &str) {
    let lower = word.to_lowercase();
    let keeps =
        lower.chars().count() <= MAX_WORD_CHARACTERS && !STOP_WORDS.contains(&lower.as_str());
    if keeps {
        found.push(stem(&lower));
    }
}

/// Adds the words of a run of letters, digits and underscores that holds no
/// character of a script without spaces: the run, and an identifier's
/// parts.
fn add_run(found: &mut Vec<String>, run: &str) {
    add_word(found, run);
    let parts = identifier_parts(run);
    if parts.len() > 1 {
        for part in parts {
            add_word(found, part);
        }
    }
}

/// Adds the pairs of neighbouring characters of a run of a script without
/// spaces, or its one character, so that words of two characters match.
fn add_pairs(found: &mut Vec<String>, run: &[char]) {
    if let [single] = run {
        found.push(single.to_string());
        return;
    }
    for pair in run.windows(2) {
        found.push(pair.iter().collect());
    }
}

/// The words of a text as a search ranks them, after the earlier harness:
/// lower-case runs of letters, digits and underscores without common English
/// words and with plurals folded, an identifier's parts as well, and the
/// pairs of characters of scripts written without spaces.
pub(crate) fn words(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut run = String::new();
    let mut spaceless: Vec<char> = Vec::new();
    let flush_run = |found: &mut Vec<String>, run: &mut String| {
        if !run.is_empty() {
            add_run(found, run);
            run.clear();
        }
    };
    let flush_spaceless = |found: &mut Vec<String>, spaceless: &mut Vec<char>| {
        if !spaceless.is_empty() {
            add_pairs(found, spaceless);
            spaceless.clear();
        }
    };
    for character in text.chars() {
        if without_spaces(character) {
            flush_run(&mut found, &mut run);
            spaceless.push(character);
        } else if character.is_alphanumeric() || character == '_' {
            flush_spaceless(&mut found, &mut spaceless);
            run.push(character);
        } else {
            flush_run(&mut found, &mut run);
            flush_spaceless(&mut found, &mut spaceless);
        }
    }
    flush_run(&mut found, &mut run);
    flush_spaceless(&mut found, &mut spaceless);
    found
}

/// A vector scaled to a length of 1, or `None` for one without length.
pub(crate) fn normalized(mut vector: Vec<f32>) -> Option<Vec<f32>> {
    let length = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    let has_length = length.is_finite() && length > 0.0;
    if !has_length {
        return None;
    }
    for value in &mut vector {
        *value /= length;
    }
    Some(vector)
}

/// A notebook's passages with their words, ready to search.
#[derive(Debug)]
pub(crate) struct Corpus {
    passages: Vec<CorpusRow>,
    /// Each word's passages, with how often the word occurs in each.
    postings: HashMap<String, Vec<(usize, u32)>>,
    /// How many words each passage has.
    lengths: Vec<u32>,
    average_length: f32,
    /// About how many bytes the corpus takes in memory.
    bytes: usize,
}

impl Corpus {
    /// The corpus of these passages.
    pub(crate) fn new(passages: Vec<CorpusRow>) -> Self {
        let mut postings: HashMap<String, Vec<(usize, u32)>> = HashMap::new();
        let mut lengths = Vec::with_capacity(passages.len());
        let mut bytes = 0;
        for (index, passage) in passages.iter().enumerate() {
            let passage_words = words(&passage.text);
            lengths.push(u32::try_from(passage_words.len()).unwrap_or(u32::MAX));
            let mut counts: HashMap<String, u32> = HashMap::new();
            for word in passage_words {
                *counts.entry(word).or_insert(0) += 1;
            }
            for (word, count) in counts {
                bytes += word.len() + 16;
                postings.entry(word).or_default().push((index, count));
            }
            let vector_bytes = passage.vector.as_ref().map_or(0, |vector| vector.len() * 4);
            bytes += passage.text.len() + passage.source_name.len() + vector_bytes + 96;
        }
        let total: u64 = lengths.iter().map(|length| u64::from(*length)).sum();
        // A corpus of a few million words averages exactly enough.
        #[expect(
            clippy::cast_precision_loss,
            reason = "the average length of a passage needs no more than an f32's precision"
        )]
        let average_length = if lengths.is_empty() {
            0.0
        } else {
            total as f32 / lengths.len() as f32
        };
        Self {
            passages,
            postings,
            lengths,
            average_length,
            bytes,
        }
    }

    /// Whether the corpus has no passage.
    pub(crate) fn is_empty(&self) -> bool {
        self.passages.is_empty()
    }

    /// About how many bytes the corpus takes in memory.
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// The passage at an index.
    pub(crate) fn passage(&self, index: usize) -> Option<&CorpusRow> {
        self.passages.get(index)
    }

    /// The passages nearest to the query's vector, best first, at most
    /// [`CANDIDATES`], of those that `keep` takes. A passage without a vector
    /// of the current model is left out.
    pub(crate) fn by_meaning(
        &self,
        query: &[f32],
        keep: &dyn Fn(&CorpusRow) -> bool,
    ) -> Vec<usize> {
        let mut scored: Vec<(usize, f32)> = self
            .passages
            .iter()
            .enumerate()
            .filter(|(_index, passage)| keep(passage))
            .filter_map(|(index, passage)| {
                let vector = passage.vector.as_ref()?;
                if vector.len() != query.len() {
                    return None;
                }
                let cosine: f32 = vector
                    .iter()
                    .zip(query)
                    .map(|(left, right)| left * right)
                    .sum();
                Some((index, cosine))
            })
            .collect();
        best_first(&mut scored);
        scored
            .into_iter()
            .take(CANDIDATES)
            .map(|(index, _score)| index)
            .collect()
    }

    /// The passages whose words best match the query's by BM25, best first,
    /// at most [`CANDIDATES`], of those that `keep` takes. A passage without
    /// any of the query's words is left out.
    pub(crate) fn by_words(&self, query: &str, keep: &dyn Fn(&CorpusRow) -> bool) -> Vec<usize> {
        let mut asked = words(query);
        asked.sort_unstable();
        asked.dedup();
        #[expect(
            clippy::cast_precision_loss,
            reason = "the counts of passages need no more than an f32's precision"
        )]
        let total = self.passages.len() as f32;
        let mut scores: HashMap<usize, f32> = HashMap::new();
        for word in &asked {
            let Some(postings) = self.postings.get(word) else {
                continue;
            };
            #[expect(
                clippy::cast_precision_loss,
                reason = "the counts of passages need no more than an f32's precision"
            )]
            let holding = postings.len() as f32;
            let rarity = (1.0 + (total - holding + 0.5) / (holding + 0.5)).ln();
            for (index, count) in postings {
                if !keep(&self.passages[*index]) {
                    continue;
                }
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "the counts of words need no more than an f32's precision"
                )]
                let (count, length) = (*count as f32, self.lengths[*index] as f32);
                let relative = if self.average_length > 0.0 {
                    length / self.average_length
                } else {
                    1.0
                };
                let saturated = count * (BM25_K1 + 1.0)
                    / (count + BM25_K1 * (1.0 - BM25_B + BM25_B * relative));
                *scores.entry(*index).or_insert(0.0) += rarity * saturated;
            }
        }
        let mut ranked: Vec<(usize, f32)> = scores.into_iter().collect();
        best_first(&mut ranked);
        ranked
            .into_iter()
            .take(CANDIDATES)
            .map(|(index, _score)| index)
            .collect()
    }
}

/// Sorts by score, the best first, and keeps equal scores in the order of
/// their passages, so that a search gives the same order every time.
fn best_first<T: Ord>(scored: &mut [(T, f32)]) {
    scored.sort_by(|(left_index, left), (right_index, right)| {
        right
            .total_cmp(left)
            .then_with(|| left_index.cmp(right_index))
    });
}

/// Fuses rankings by reciprocal rank: each passage scores 1 / (60 + its rank)
/// in each ranking that holds it, and the best total comes first.
pub(crate) fn fuse<T: Copy + Eq + Hash + Ord>(rankings: &[Vec<T>]) -> Vec<T> {
    let mut scores: HashMap<T, f32> = HashMap::new();
    for ranking in rankings {
        for (rank, index) in ranking.iter().enumerate() {
            #[expect(
                clippy::cast_precision_loss,
                reason = "a rank below a hundred needs no more than an f32's precision"
            )]
            let reciprocal = 1.0 / (FUSION_K + rank as f32 + 1.0);
            *scores.entry(*index).or_insert(0.0) += reciprocal;
        }
    }
    let mut ranked: Vec<(T, f32)> = scores.into_iter().collect();
    best_first(&mut ranked);
    ranked.into_iter().map(|(index, _score)| index).collect()
}

/// Whether two passages share more than half of the shorter one's text: they
/// are of one section of one source and their byte ranges overlap that much.
fn mostly_shared(left: &CorpusRow, right: &CorpusRow) -> bool {
    let same_section = left.source_id == right.source_id && left.section == right.section;
    if !same_section {
        return false;
    }
    let start = left.start.max(right.start);
    let end = left.end.min(right.end);
    let shared = end.saturating_sub(start);
    let shorter = (left.end - left.start).min(right.end - right.start);
    shared * 2 > shorter
}

/// The passages that go, from the ranked ones, best first: at most
/// `max_items`, whose texts together stay within `max_bytes`, without a
/// passage that shares most of its text with one that goes.
pub(crate) fn choose_rows(
    ranked: &[&CorpusRow],
    max_items: usize,
    max_bytes: usize,
) -> Vec<CorpusRow> {
    let mut chosen: Vec<CorpusRow> = Vec::new();
    let mut room = max_bytes;
    for passage in ranked {
        if chosen.len() == max_items {
            break;
        }
        if passage.text.len() > room {
            continue;
        }
        if chosen.iter().any(|kept| mostly_shared(kept, passage)) {
            continue;
        }
        room -= passage.text.len();
        chosen.push((*passage).clone());
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::{CANDIDATES, Corpus, choose_rows, fuse, normalized, words};
    use crate::commands::SectionPlace;
    use crate::store::CorpusRow;

    /// Keeps every passage.
    fn all(_row: &CorpusRow) -> bool {
        true
    }

    #[test]
    fn a_search_ranks_only_the_passages_that_it_keeps() {
        let corpus = Corpus::new(vec![
            row("a", 0, "light bends", Some(vec![1.0])),
            row("b", 1, "light bends", Some(vec![1.0])),
        ]);
        let only_b = |row: &CorpusRow| row.source_id == "b";

        assert_eq!(corpus.by_words("light", &only_b), [1]);
        assert_eq!(corpus.by_meaning(&[1.0], &only_b), [1]);
    }

    fn row(source: &str, number: u32, text: &str, vector: Option<Vec<f32>>) -> CorpusRow {
        CorpusRow {
            source_id: source.to_owned(),
            source_name: format!("{source}.md"),
            number,
            section: 0,
            start: number * 100,
            end: number * 100 + u32::try_from(text.len()).unwrap(),
            text: text.to_owned(),
            place: SectionPlace::Whole,
            vector,
        }
    }

    #[test]
    fn words_are_lower_case_without_common_words_and_with_plurals_folded() {
        assert_eq!(
            words("How does the Light BEND: 3x, Ünïcode policies and hosts!"),
            ["light", "bend", "3x", "ünïcode", "policy", "host"]
        );
        // "class" keeps its double s, and short words keep theirs.
        assert_eq!(words("class gas"), ["class", "gas"]);
        assert_eq!(words(" — "), Vec::<String>::new());
    }

    #[test]
    fn identifiers_give_their_parts_as_well() {
        assert_eq!(
            words("HTTPServer_port"),
            ["httpserver_port", "http", "server", "port"]
        );
        assert_eq!(words("getUserName"), ["getusername", "get", "user", "name"]);
    }

    #[test]
    fn scripts_without_spaces_give_pairs_of_characters() {
        assert_eq!(
            words("光は曲がる light"),
            ["光は", "は曲", "曲が", "がる", "light"]
        );
        assert_eq!(words("光 光"), ["光", "光"]);
        assert_eq!(words("Сервер SSH"), ["сервер", "ssh"]);
    }

    #[test]
    fn vectors_are_scaled_to_a_length_of_one_and_one_without_length_is_none() {
        let vector = normalized(vec![3.0, 4.0]).unwrap();

        assert!((vector[0] - 0.6).abs() < 1e-6);
        assert!((vector[1] - 0.8).abs() < 1e-6);
        assert_eq!(normalized(vec![0.0, 0.0]), None);
        assert_eq!(normalized(vec![f32::NAN, 1.0]), None);
    }

    #[test]
    fn meaning_ranks_by_the_cosine_and_leaves_out_passages_without_a_vector() {
        let corpus = Corpus::new(vec![
            row("a", 0, "first", Some(vec![1.0, 0.0])),
            row("a", 1, "second", Some(vec![0.6, 0.8])),
            row("a", 2, "third", None),
            row("a", 3, "fourth", Some(vec![0.0, 1.0, 0.0])),
        ]);

        assert_eq!(corpus.by_meaning(&[0.0, 1.0], &all), [1, 0]);
    }

    #[test]
    fn words_rank_rare_matches_higher_and_leave_out_passages_without_them() {
        let corpus = Corpus::new(vec![
            row("a", 0, "the light and the water", None),
            row("a", 1, "refraction bends light", None),
            row("a", 2, "the bread and the flour", None),
            row("a", 3, "the the the", None),
        ]);

        let ranked = corpus.by_words("Why does refraction bend the light?", &all);
        let rank_of = |passage: usize| ranked.iter().position(|index| *index == passage);

        // "refraction" is rare, so the passage that holds it, "bends" for
        // "bend" and "light" comes first. "the" is a common word that no
        // passage matches by, so those with only common words are left out.
        assert_eq!(ranked.first(), Some(&1));
        assert!(rank_of(0).is_some(), "{ranked:?}");
        assert_eq!(rank_of(2), None);
        assert_eq!(rank_of(3), None);
        assert_eq!(
            corpus.by_words("nothing matches", &all),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn a_ranking_keeps_at_most_the_candidates() {
        let passages = (0..80)
            .map(|number| row("a", number, "light", Some(vec![1.0])))
            .collect();
        let corpus = Corpus::new(passages);

        assert_eq!(corpus.by_meaning(&[1.0], &all).len(), CANDIDATES);
        assert_eq!(corpus.by_words("light", &all).len(), CANDIDATES);
    }

    #[test]
    fn fusion_puts_first_what_ranks_well_in_both() {
        let fused = fuse(&[vec![1, 2, 3], vec![3, 1, 4]]);

        assert_eq!(fused[0], 1);
        assert_eq!(fused[1], 3);
        assert_eq!(fused.len(), 4);
    }

    #[test]
    fn the_choice_keeps_the_room_the_number_and_no_two_that_share_most_text() {
        let mut overlapping = row("a", 1, &"x".repeat(80), None);
        overlapping.start = 20;
        overlapping.end = 100;
        let rows = [
            row("a", 0, &"y".repeat(100), None),
            overlapping,
            row("b", 0, &"z".repeat(100), None),
            row("c", 0, &"w".repeat(100), None),
        ];
        let ranked: Vec<&CorpusRow> = rows.iter().collect();

        let chosen = choose_rows(&ranked, 2, 1_000);
        let tight = choose_rows(&ranked, 6, 150);

        let sources: Vec<&str> = chosen
            .iter()
            .map(|passage| passage.source_id.as_str())
            .collect();
        assert_eq!(sources, ["a", "b"]);
        assert_eq!(tight.len(), 1);
    }
}
