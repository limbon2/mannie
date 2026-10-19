//! The SSH server of the plugin's end-to-end tests: it prints one line of
//! JSON with its port, its account and the fingerprint of its host key, then
//! serves until the test ends it. It is an example of the test-only crate,
//! so no release ships it.

use mannie_ssh_testing::{Options, PASSWORD, TestServer, USERNAME};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let server = TestServer::start(Options::default()).await?;
    let line = serde_json::json!({
        "port": server.port(),
        "username": USERNAME,
        "password": PASSWORD,
        "fingerprint": server.fingerprint(),
    });
    print_line(&line.to_string());
    std::future::pending::<()>().await;
    Ok(())
}

#[expect(
    clippy::print_stdout,
    reason = "the end-to-end tests read the server's address from its first line"
)]
fn print_line(line: &str) {
    println!("{line}");
}
