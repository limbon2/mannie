//! The keys of a terminal's commands and selections, which cite links
//! (`cite:terminal/<key>`, ADR-0049, decision 4) and mentions name.
//!
//! A key starts with its terminal's own random ID, so a key of a terminal
//! that ended never names a command of a later terminal with the same
//! number. A command's key is `<terminal>.<command>`, such as `3f9a2c1d.5`;
//! a selection's is `<terminal>.s<random>`. A cite link may add a line
//! fragment, `#L3-L4` (ADR-0054, decision 2), which the key itself never
//! holds.

use mannie_plugin_sdk::points::context::cite_link;

/// The provider that the plugin's cite links and mentions name.
pub(crate) const PROVIDER: &str = "terminal";

/// The characters of a terminal's ID.
const ID_CHARS: usize = 8;

/// The last 8 hexadecimal digits of a new UUIDv7, which are random.
fn random_digits() -> String {
    let id = uuid::Uuid::now_v7().simple().to_string();
    id[id.len() - ID_CHARS..].to_owned()
}

/// A new terminal's ID: 8 random hexadecimal digits.
pub(crate) fn new_terminal_id() -> String {
    random_digits()
}

/// The key of a command.
pub(crate) fn block_key(terminal: &str, block: u32) -> String {
    format!("{terminal}.{block}")
}

/// A new key for a selection of a terminal.
pub(crate) fn selection_key(terminal: &str) -> String {
    format!("{terminal}.s{}", random_digits())
}

/// What a key names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Key<'key> {
    /// A command of a terminal.
    Block {
        /// The terminal's ID.
        terminal: &'key str,
        /// The command's number.
        number: u32,
    },
    /// A selection of a terminal.
    Selection {
        /// The terminal's ID.
        terminal: &'key str,
    },
}

/// What a key names, or `None` for a key that the plugin never made. The key
/// comes from the webview or from the model, so it is checked as untrusted
/// input.
pub(crate) fn parse(key: &str) -> Option<Key<'_>> {
    let (terminal, rest) = key.split_once('.')?;
    let is_id = terminal.len() == ID_CHARS
        && terminal
            .chars()
            .all(|character| character.is_ascii_hexdigit());
    if !is_id {
        return None;
    }
    if let Some(random) = rest.strip_prefix('s') {
        let is_random = random.len() == ID_CHARS
            && random
                .chars()
                .all(|character| character.is_ascii_hexdigit());
        return is_random.then_some(Key::Selection { terminal });
    }
    let number: u32 = rest.parse().ok()?;
    (number > 0 && rest == number.to_string()).then_some(Key::Block { terminal, number })
}

/// The Markdown link that cites a command, which the model copies.
pub(crate) fn block_link(key: &str, label: &str) -> String {
    cite_link(PROVIDER, key, label)
}

#[cfg(test)]
mod tests {
    use super::{Key, block_key, new_terminal_id, parse, selection_key};

    #[test]
    fn keys_name_what_the_plugin_made_and_nothing_else() {
        let terminal = new_terminal_id();
        let block = block_key(&terminal, 5);
        let selection = selection_key(&terminal);

        assert_eq!(
            parse(&block),
            Some(Key::Block {
                terminal: &terminal,
                number: 5
            })
        );
        assert_eq!(
            parse(&selection),
            Some(Key::Selection {
                terminal: &terminal
            })
        );
        for key in [
            "",
            "5",
            "zzzzzzzz.5",
            "3f9a2c1d.0",
            "3f9a2c1d.05",
            "3f9a2c1d.5#L1",
            "3f9a2c1d.sx",
            "3f9a2c1d",
        ] {
            assert_eq!(parse(key), None, "{key}");
        }
    }
}
