//! The limits of results and the helpers that keep renderings within them
//! (docs/features/system-tools/tools-and-results.md, results and limits).

use std::time::SystemTime;

use time::OffsetDateTime;
use time::macros::format_description;

/// The most bytes of a rendering that the model reads.
pub const RESULT_LIMIT_BYTES: usize = 64 * 1024;

/// The fewest bytes of a rendering, whatever the context budget: room for a
/// header and some content.
pub const MIN_RESULT_LIMIT_BYTES: usize = 4 * 1024;

/// The bytes that a page keeps free for its header and its last line.
pub(crate) const FRAME_BYTES: usize = 512;

/// The result limit for a context budget in bytes: 64 KiB, or a quarter of the
/// budget when that is smaller.
pub fn result_limit(context_budget_bytes: usize) -> usize {
    (context_budget_bytes / 4).clamp(MIN_RESULT_LIMIT_BYTES, RESULT_LIMIT_BYTES)
}

/// The body of a page: lines added while they fit in the result limit, after
/// room for the header, which names `fixed`, such as a path.
#[derive(Debug)]
pub(crate) struct Page {
    body: String,
    budget: usize,
    full: bool,
}

impl Page {
    /// A page within `limit` bytes, whose header holds the texts in `fixed`.
    pub(crate) fn new(limit: usize, fixed: &[&str]) -> Self {
        let fixed_bytes: usize = fixed.iter().map(|text| text.len()).sum();
        Self {
            body: String::new(),
            budget: limit.saturating_sub(fixed_bytes + FRAME_BYTES),
            full: false,
        }
    }

    /// Adds a line when it fits. Once a line does not fit, the page is full and
    /// takes no more lines, so a page never skips a line.
    pub(crate) fn push(&mut self, line: &str) -> bool {
        if self.full || self.body.len() + line.len() + 1 > self.budget {
            self.full = true;
            return false;
        }
        self.body.push_str(line);
        self.body.push('\n');
        true
    }

    /// The lines added, each ending in a line break.
    pub(crate) fn into_body(self) -> String {
        self.body
    }
}

/// A line cut to `max_chars` characters, and how many characters were left
/// out.
pub(crate) fn cut_line(line: &str, max_chars: usize) -> (&str, usize) {
    match line.char_indices().nth(max_chars) {
        Some((end, _)) => (&line[..end], line[end..].chars().count()),
        None => (line, 0),
    }
}

/// A time of the file system in UTC, to the second, as RFC 3339.
pub(crate) fn format_time(time: SystemTime) -> String {
    let format = format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");
    OffsetDateTime::from(time)
        .format(&format)
        .unwrap_or_else(|_unformattable| "unknown".to_owned())
}

/// A count as a `u32` for an outcome, saturating at its largest value.
pub(crate) fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::{
        MIN_RESULT_LIMIT_BYTES, Page, RESULT_LIMIT_BYTES, cut_line, format_time, result_limit,
    };

    #[test]
    fn the_limit_is_a_quarter_of_a_small_context() {
        assert_eq!(result_limit(1_000_000), RESULT_LIMIT_BYTES);
        assert_eq!(result_limit(100_000), 25_000);
        assert_eq!(result_limit(1_000), MIN_RESULT_LIMIT_BYTES);
    }

    #[test]
    fn a_page_stops_at_the_first_line_that_does_not_fit() {
        let mut page = Page::new(600, &["header"]);

        assert!(page.push("short"));
        assert!(!page.push(&"long".repeat(100)));
        assert!(!page.push("short"));
        assert_eq!(page.into_body(), "short\n");
    }

    #[test]
    fn long_lines_are_cut_at_a_character() {
        assert_eq!(cut_line("abcdef", 4), ("abcd", 2));
        assert_eq!(cut_line("äöü", 2), ("äö", 1));
        assert_eq!(cut_line("abc", 3), ("abc", 0));
    }

    #[test]
    fn times_are_utc_to_the_second() {
        let time = UNIX_EPOCH + Duration::from_millis(1_760_000_000_123);

        assert_eq!(format_time(time), "2025-10-09T08:53:20Z");
    }
}
