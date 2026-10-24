//! The MCP server of the plugin's tests over stdio: the server of the
//! current era, or with `--legacy` the scripted server of the 2025-06-18
//! era. It serves until the client closes its input. It is an example of the
//! test-only crate, so no release ships it.

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let legacy = std::env::args().any(|argument| argument == "--legacy");
    if legacy {
        mannie_mcp_testing::legacy::serve().await
    } else {
        mannie_mcp_testing::serve_stdio().await
    }
}
