//! The output of `git blame --porcelain`: for each line, the commit that
//! last changed it, with the commit's author, time and summary given once,
//! at the commit's first line, and the line's text after a tab. Lines in a
//! row from one commit become one run.

use std::collections::HashMap;

use serde::Serialize;

/// The hash of lines that no commit holds yet.
const NOT_COMMITTED: &str = "0000000000000000000000000000000000000000";

/// A commit as blame names it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BlameCommit {
    pub(crate) oid: String,
    pub(crate) author: String,
    /// The day the author wrote it, in the author's time zone, such as
    /// `2026-10-19`.
    pub(crate) day: String,
    pub(crate) summary: String,
    /// Whether the lines are changes that no commit holds yet.
    pub(crate) uncommitted: bool,
}

/// A line of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BlameLine {
    /// Its number in the file.
    pub(crate) number: u32,
    pub(crate) text: String,
}

/// Lines in a row that one commit last changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BlameRun {
    pub(crate) commit: BlameCommit,
    pub(crate) lines: Vec<BlameLine>,
}

/// The day of a time in seconds since 1970 in a zone such as `+0200`,
/// as `YYYY-MM-DD`.
fn day_of(seconds: i64, zone: &str) -> Option<String> {
    let sign = match zone.get(..1)? {
        "+" => 1,
        "-" => -1,
        _other => return None,
    };
    let hours: i64 = zone.get(1..3)?.parse().ok()?;
    let minutes: i64 = zone.get(3..5)?.parse().ok()?;
    let local = seconds + sign * (hours * 3600 + minutes * 60);
    let (year, month, day) = civil_from_days(local.div_euclid(86_400));
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// The date of a day counted from 1970-01-01, in the proleptic Gregorian
/// calendar (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// What blame said of each commit so far, and the time and zone of the
/// commit that it describes now.
#[derive(Default)]
struct Commits {
    known: HashMap<String, BlameCommit>,
    time: Option<i64>,
}

/// Reads the output of `git blame --porcelain` into runs.
pub(crate) fn parse_blame(output: &str) -> Vec<BlameRun> {
    let mut commits = Commits::default();
    let mut runs: Vec<BlameRun> = Vec::new();
    let mut current: Option<(String, u32)> = None;
    for line in output.lines() {
        if let Some(text) = line.strip_prefix('\t') {
            let Some((oid, number)) = current.take() else {
                continue;
            };
            let commit = commits
                .known
                .get(&oid)
                .cloned()
                .unwrap_or_else(|| BlameCommit {
                    oid: oid.clone(),
                    ..BlameCommit::default()
                });
            let line = BlameLine {
                number,
                text: text.to_owned(),
            };
            match runs.last_mut() {
                Some(run) if run.commit.oid == oid => run.lines.push(line),
                _other => runs.push(BlameRun {
                    commit,
                    lines: vec![line],
                }),
            }
            continue;
        }
        let mut parts = line.splitn(2, ' ');
        let (key, value) = (
            parts.next().unwrap_or_default(),
            parts.next().unwrap_or_default(),
        );
        let is_header =
            key.len() >= 40 && key.chars().all(|character| character.is_ascii_hexdigit());
        if is_header {
            let mut numbers = value.split(' ');
            let final_line = numbers.nth(1).and_then(|number| number.parse().ok());
            if let Some(final_line) = final_line {
                current = Some((key.to_owned(), final_line));
                commits
                    .known
                    .entry(key.to_owned())
                    .or_insert_with(|| BlameCommit {
                        oid: key.to_owned(),
                        uncommitted: key == NOT_COMMITTED,
                        ..BlameCommit::default()
                    });
            }
            continue;
        }
        let Some((oid, _number)) = &current else {
            continue;
        };
        let Some(commit) = commits.known.get_mut(oid) else {
            continue;
        };
        match key {
            "author" => value.clone_into(&mut commit.author),
            "author-time" => commits.time = value.parse().ok(),
            "author-tz" => {
                if let Some(day) = commits.time.and_then(|time| day_of(time, value)) {
                    commit.day = day;
                }
            }
            "summary" => value.clone_into(&mut commit.summary),
            _other => {}
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::{civil_from_days, day_of, parse_blame};

    const OUTPUT: &str = "\
1a2b3c4d5e6f7a8b9c0d1a2b3c4d5e6f7a8b9c0d 10 10 2
author Jane Doe
author-mail <jane@example.com>
author-time 1760882531
author-tz +0200
committer Jane Doe
summary Fix the parser
filename src/lib.rs
\tfn parse() {
1a2b3c4d5e6f7a8b9c0d1a2b3c4d5e6f7a8b9c0d 11 11
\t    let a = 1;
0000000000000000000000000000000000000000 12 12 1
author Not Committed Yet
author-time 1760900000
author-tz +0000
summary Version of src/lib.rs from src/lib.rs
filename src/lib.rs
\t    let b = 2;
1a2b3c4d5e6f7a8b9c0d1a2b3c4d5e6f7a8b9c0d 13 13 1
\t}
";

    #[test]
    fn lines_in_a_row_from_one_commit_form_a_run() {
        let runs = parse_blame(OUTPUT);

        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].commit.author, "Jane Doe");
        assert_eq!(runs[0].commit.summary, "Fix the parser");
        assert_eq!(runs[0].commit.day, "2025-10-19");
        assert_eq!(
            runs[0]
                .lines
                .iter()
                .map(|line| (line.number, line.text.as_str()))
                .collect::<Vec<_>>(),
            [(10, "fn parse() {"), (11, "    let a = 1;")]
        );
        assert!(runs[1].commit.uncommitted);
        assert_eq!(runs[2].commit.summary, "Fix the parser");
        assert_eq!(runs[2].lines[0].number, 13);
    }

    #[test]
    fn days_follow_the_author_s_time_zone() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(20_381), (2025, 10, 20));
        // 23:30 UTC is the next day two hours east.
        assert_eq!(
            day_of(1_760_916_600, "+0000").as_deref(),
            Some("2025-10-19")
        );
        assert_eq!(
            day_of(1_760_916_600, "+0200").as_deref(),
            Some("2025-10-20")
        );
        assert_eq!(
            day_of(1_760_916_600, "-0500").as_deref(),
            Some("2025-10-19")
        );
        assert_eq!(day_of(0, "zone"), None);
    }
}
