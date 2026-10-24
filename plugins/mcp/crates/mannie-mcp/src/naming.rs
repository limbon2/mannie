//! The names that agents call the servers' tools by (plugins/mcp/docs/
//! README.md, tools for agents): `mcp_<prefix>_<tool>`, with the server's
//! prefix and its tool's name in snake case, within the characters that a
//! tool's name may have. A name that would be too long, or that another tool
//! of the server would get too, is cut and ends with a hash of the server's
//! own name for the tool, so it stays the same from one listing to the next.

use std::fmt::Write as _;

use mannie_plugin_sdk::points::tools::MAX_TOOL_NAME_CHARS;
use sha2::{Digest, Sha256};

/// The start of every tool that the plugin publishes.
pub(crate) const TOOL_PREFIX: &str = "mcp_";

/// The tool that reads a resource of any server.
pub(crate) const READ_RESOURCE: &str = "mcp_read_resource";

/// The prefixes that no server may take, since a tool of the plugin's own
/// starts with them.
pub(crate) const RESERVED_PREFIXES: [&str; 1] = ["read"];

/// The most characters of a server's prefix.
pub(crate) const MAX_PREFIX_CHARS: usize = 16;

/// The hexadecimal digits of the hash that ends a cut name.
const HASH_CHARS: usize = 8;

/// Whether a prefix is one that a server may have: 1 to
/// [`MAX_PREFIX_CHARS`] lower-case ASCII letters and digits, starting with a
/// letter, and none of [`RESERVED_PREFIXES`].
pub(crate) fn is_prefix(prefix: &str) -> bool {
    let well_formed = prefix.len() <= MAX_PREFIX_CHARS
        && prefix
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_lowercase())
        && prefix
            .chars()
            .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit());
    well_formed && !RESERVED_PREFIXES.contains(&prefix)
}

/// The prefix that a server's name suggests, which no prefix of `taken` is:
/// the name's ASCII letters and digits in lower case, without its leading
/// digits, cut to fit, and a number after it when another server has it. A
/// name without such letters suggests `server`.
pub(crate) fn suggested_prefix(name: &str, taken: &[String]) -> String {
    let letters: String = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .skip_while(char::is_ascii_digit)
        .take(MAX_PREFIX_CHARS)
        .collect();
    let base = if letters.is_empty() || RESERVED_PREFIXES.contains(&letters.as_str()) {
        "server".to_owned()
    } else {
        letters
    };
    let is_free = |candidate: &str| !taken.iter().any(|prefix| prefix == candidate);
    if is_free(&base) {
        return base;
    }
    // Among one more candidate than there are prefixes, one is free.
    let candidates = u32::try_from(taken.len())
        .unwrap_or(u32::MAX)
        .saturating_add(2);
    (2..=candidates)
        .map(|number| {
            let suffix = number.to_string();
            let room = MAX_PREFIX_CHARS - suffix.len();
            let start: String = base.chars().take(room).collect();
            format!("{start}{suffix}")
        })
        .find(|candidate| is_free(candidate))
        .unwrap_or(base)
}

/// The snake case of a tool's name: ASCII letters and digits in lower case,
/// with the words of camel case apart, every other character as an
/// underscore, and no repeated, leading or trailing underscores. A name
/// without letters or digits becomes `tool`.
pub(crate) fn snake_case(name: &str) -> String {
    let mut snake = String::with_capacity(name.len());
    let mut after_lower = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            if after_lower && character.is_ascii_uppercase() {
                snake.push('_');
            }
            after_lower = character.is_ascii_lowercase() || character.is_ascii_digit();
            snake.push(character.to_ascii_lowercase());
        } else {
            after_lower = false;
            if !snake.is_empty() && !snake.ends_with('_') {
                snake.push('_');
            }
        }
    }
    let trimmed = snake.trim_end_matches('_');
    if trimmed.is_empty() {
        "tool".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// The hash that ends a cut name: the first hexadecimal digits of the
/// SHA-256 of the server's own name for the tool.
fn hash_of(name: &str) -> String {
    let digest = Sha256::digest(name.as_bytes());
    let mut hash = String::with_capacity(HASH_CHARS);
    for byte in digest.iter().take(HASH_CHARS / 2) {
        let _written = write!(hash, "{byte:02x}");
    }
    hash
}

/// The name of a tool, cut so that its hash fits after it.
fn hashed(start: &str, name: &str) -> String {
    let room = MAX_TOOL_NAME_CHARS - HASH_CHARS - 1;
    let cut: String = start.chars().take(room).collect();
    format!("{}_{}", cut.trim_end_matches('_'), hash_of(name))
}

/// The model's names of a server's tools, in the order of `tools`, the
/// server's own names: `mcp_<prefix>_<tool>`. A name past
/// [`MAX_TOOL_NAME_CHARS`], or one that an earlier tool got, is cut and ends
/// with its hash.
pub(crate) fn model_names(prefix: &str, tools: &[&str]) -> Vec<String> {
    let mut names: Vec<String> = Vec::with_capacity(tools.len());
    for tool in tools {
        let plain = format!("{TOOL_PREFIX}{prefix}_{}", snake_case(tool));
        let fits = plain.len() <= MAX_TOOL_NAME_CHARS;
        let free = !names.contains(&plain);
        let name = if fits && free {
            plain
        } else {
            hashed(&plain, tool)
        };
        names.push(name);
    }
    names
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::points::tools::{MAX_TOOL_NAME_CHARS, is_tool_name};

    use super::{is_prefix, model_names, snake_case, suggested_prefix};

    #[test]
    fn a_prefix_is_short_lower_case_and_not_reserved() {
        for prefix in ["github", "fs", "a1", "linear2"] {
            assert!(is_prefix(prefix), "{prefix}");
        }
        for prefix in [
            "",
            "GitHub",
            "1github",
            "git-hub",
            "read",
            "abcdefghijklmnopq",
        ] {
            assert!(!is_prefix(prefix), "{prefix}");
        }
    }

    #[test]
    fn a_name_suggests_its_letters_and_a_number_when_taken() {
        assert_eq!(suggested_prefix("GitHub", &[]), "github");
        assert_eq!(suggested_prefix("2nd Brain", &[]), "ndbrain");
        assert_eq!(suggested_prefix("Ünicode ★", &[]), "nicode");
        assert_eq!(suggested_prefix("★", &[]), "server");
        assert_eq!(suggested_prefix("Read", &[]), "server");
        assert_eq!(
            suggested_prefix("GitHub", &["github".to_owned(), "github2".to_owned()]),
            "github3"
        );
        let long = suggested_prefix("A very long server name", &["averylongservern".to_owned()]);
        assert_eq!(long, "averylongserver2");
        assert!(is_prefix(&long));
    }

    #[test]
    fn a_tool_name_becomes_snake_case() {
        assert_eq!(snake_case("create_issue"), "create_issue");
        assert_eq!(snake_case("get-annotated-message"), "get_annotated_message");
        assert_eq!(snake_case("searchRepositories"), "search_repositories");
        assert_eq!(snake_case("HTTPRequest"), "httprequest");
        assert_eq!(snake_case("read file (v2)"), "read_file_v2");
        assert_eq!(snake_case("__private__"), "private");
        assert_eq!(snake_case("★"), "tool");
    }

    #[test]
    fn the_names_of_a_servers_tools_fit_and_stay_apart() {
        let long = "x".repeat(80);
        let names = model_names("github", &["create_issue", "create-issue", &long, "★", "☆"]);

        assert_eq!(names[0], "mcp_github_create_issue");
        assert!(
            names[1].starts_with("mcp_github_create_issue_"),
            "{names:?}"
        );
        assert_ne!(names[0], names[1]);
        assert_eq!(names[2].len(), MAX_TOOL_NAME_CHARS);
        assert_eq!(names[3], "mcp_github_tool");
        assert_ne!(names[3], names[4]);
        for name in &names {
            assert!(is_tool_name(name), "{name}");
        }
        // The same tools get the same names again.
        assert_eq!(
            model_names("github", &["create_issue", "create-issue", &long, "★", "☆"]),
            names
        );
    }
}
