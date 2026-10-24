//! Calls of a tool server's tools (ADR-0043, decision 4): where the server
//! runs, as the owner registered it, and what the server says of its tools.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Where a tool server runs, as the owner registered it with a plugin: a
/// program that Mannie starts, or a URL. It never holds a credential, and a
/// changed endpoint is another server, which an earlier grant does not cover.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ToolServerEndpoint {
    /// A program that Mannie starts, with its arguments exactly as the
    /// program receives them.
    Program {
        /// The program, as the owner named it: an absolute path or a name
        /// that the core finds on Mannie's `PATH`.
        program: String,
        /// Its arguments.
        arguments: Vec<String>,
    },
    /// A URL, without user information.
    Url {
        /// The URL.
        url: String,
    },
}

/// What a tool server says of one of its tools. The approval shows it as the
/// server's claim, and nothing that authorizes a call reads it (ADR-0043,
/// decision 4).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "snake_case")]
pub enum ToolHint {
    /// The server says that the tool changes nothing.
    ReadOnly,
    /// The server says that the tool may destroy or overwrite.
    Destructive,
    /// The server says that a second call with the same arguments changes
    /// nothing more.
    Idempotent,
    /// The server says that the tool reaches beyond the server, such as the
    /// web.
    OpenWorld,
}

#[cfg(test)]
mod tests {
    use super::{ToolHint, ToolServerEndpoint};

    #[test]
    fn an_endpoint_and_a_hint_cross_json_by_their_kinds() {
        let program = ToolServerEndpoint::Program {
            program: "npx".to_owned(),
            arguments: vec![
                "-y".to_owned(),
                "@modelcontextprotocol/server-github".to_owned(),
            ],
        };
        let url = ToolServerEndpoint::Url {
            url: "https://mcp.example.com/mcp".to_owned(),
        };

        assert_eq!(
            serde_json::to_value(&program).unwrap(),
            serde_json::json!({
                "kind": "program",
                "program": "npx",
                "arguments": ["-y", "@modelcontextprotocol/server-github"],
            })
        );
        assert_eq!(
            serde_json::to_value(&url).unwrap(),
            serde_json::json!({ "kind": "url", "url": "https://mcp.example.com/mcp" })
        );
        assert_eq!(
            serde_json::to_value(ToolHint::OpenWorld).unwrap(),
            serde_json::json!("open_world")
        );
    }
}
