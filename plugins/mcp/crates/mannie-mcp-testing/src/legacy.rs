//! A scripted server of the 2025-06-18 era over stdio: it answers
//! `initialize`, lists one tool and calls it, and refuses `server/discover`,
//! the request of the current era, as a server written before it does. It is
//! written by hand, without rmcp, so it stays a server of that era whatever
//! rmcp speaks.

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

/// The protocol version that the server speaks.
pub const VERSION: &str = "2025-06-18";

/// The server's name, as it introduces itself.
pub const SERVER_NAME: &str = "mannie-mcp-legacy-server";

/// The JSON-RPC code of a method that a server does not know.
const METHOD_NOT_FOUND: i64 = -32601;

/// Serves until the client closes the server's input.
pub async fn serve() -> std::io::Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut output = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(answer) = answer(&message) else {
            continue;
        };
        output.write_all(answer.to_string().as_bytes()).await?;
        output.write_all(b"\n").await?;
        output.flush().await?;
    }
    Ok(())
}

/// The answer to a message, or `None` for a notification.
fn answer(message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let result = match method {
        "initialize" => json!({
            "protocolVersion": VERSION,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": SERVER_NAME, "version": "1.0.0" },
            "instructions": "A server of 2025 that echoes."
        }),
        "ping" => json!({}),
        "tools/list" => json!({
            "tools": [{
                "name": "echo",
                "description": "Repeats the text.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "text": { "type": "string" } },
                    "required": ["text"]
                }
            }]
        }),
        "tools/call" => {
            let text = message
                .pointer("/params/arguments/text")
                .and_then(Value::as_str)
                .unwrap_or("");
            json!({ "content": [{ "type": "text", "text": format!("Echo: {text}") }], "isError": false })
        }
        _ => {
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": METHOD_NOT_FOUND, "message": format!("Method not found: {method}") }
            }));
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::answer;

    #[test]
    fn the_server_of_2025_refuses_the_request_of_the_current_era() {
        let discover =
            answer(&json!({ "jsonrpc": "2.0", "id": 1, "method": "server/discover" })).unwrap();
        let initialize =
            answer(&json!({ "jsonrpc": "2.0", "id": 2, "method": "initialize" })).unwrap();

        assert_eq!(discover["error"]["code"], json!(-32601));
        assert_eq!(initialize["result"]["protocolVersion"], json!("2025-06-18"));
        assert_eq!(
            answer(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })),
            None
        );
    }
}
