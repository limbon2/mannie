//! The output of `git grep -z -n`: one line for each match, whose name, line
//! number and text are separated by NUL. A match at a revision names the
//! revision before the path, as `<revision>:<path>`; the plugin passes
//! revisions as full hashes, which hold no colon.

use serde::Serialize;

/// A line that matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Match {
    /// The commit whose file matched, or `None` for the work tree.
    pub(crate) commit: Option<String>,
    pub(crate) path: String,
    pub(crate) line: u32,
    pub(crate) text: String,
}

/// Reads the output of `git grep -z -n`. `at_revisions` says whether the
/// search ran at revisions, whose hashes start each name.
pub(crate) fn parse_grep(output: &str, at_revisions: bool) -> Vec<Match> {
    output
        .split('\n')
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\0');
            let name = fields.next()?;
            let number = fields.next()?.parse().ok()?;
            let text = fields.next()?;
            let (commit, path) = if at_revisions {
                let (commit, path) = name.split_once(':')?;
                (Some(commit.to_owned()), path.to_owned())
            } else {
                (None, name.to_owned())
            };
            Some(Match {
                commit,
                path,
                line: number,
                text: text.trim_end_matches('\r').to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Match, parse_grep};

    #[test]
    fn matches_of_the_work_tree_name_their_path_and_line() {
        let output =
            "src/main file.rs\x0012\x00    parse(input);\nREADME.md\x003\x00Parse the input:\r\n";

        let matches = parse_grep(output, false);

        assert_eq!(
            matches,
            [
                Match {
                    commit: None,
                    path: "src/main file.rs".to_owned(),
                    line: 12,
                    text: "    parse(input);".to_owned(),
                },
                Match {
                    commit: None,
                    path: "README.md".to_owned(),
                    line: 3,
                    text: "Parse the input:".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn matches_at_revisions_name_their_commit_and_keep_colons_in_the_text() {
        let output = "1a2b3c:src/lib.rs\x007\x00let url = \"a:b\";\n";

        let matches = parse_grep(output, true);

        assert_eq!(matches[0].commit.as_deref(), Some("1a2b3c"));
        assert_eq!(matches[0].path, "src/lib.rs");
        assert_eq!(matches[0].text, "let url = \"a:b\";");
        assert_eq!(parse_grep("", true), Vec::new());
    }
}
