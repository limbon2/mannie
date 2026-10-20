//! Test tools: a scripted provider for tests of the agent loop, and a fixture
//! server that replays recorded server-sent event streams over real HTTP
//! (docs/architecture/agent-runtime.md, testing; ADR-0011, decision 7).
//!
//! Only the `testing` feature compiles this module. The desktop app and the
//! worker never enable it.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use mannie_types::provider::ApiInterface;
use reqwest::Url;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use crate::provider::{EventStream, ModelRequest, Provider, ProviderError, ProviderEvent, Target};

/// One step of a scripted call.
#[derive(Debug, Clone)]
pub enum Step {
    /// Yields an event.
    Event(ProviderEvent),
    /// Yields an error; the call ends.
    Error(ProviderError),
    /// Waits before the next step.
    Delay(Duration),
    /// Waits for a test to release the next step, without relying on timing.
    Wait(Arc<tokio::sync::Notify>),
    /// Never yields again, like a server that stops sending. Only dropping the
    /// stream ends it.
    Hang,
}

#[derive(Debug, Default)]
struct Script {
    calls: VecDeque<Vec<Step>>,
    requests: Vec<ModelRequest>,
}

/// A provider that plays scripted calls in order and records the requests,
/// so that tests of the loop need no network and no credential.
#[derive(Debug, Clone)]
pub struct ScriptedProvider {
    interface: ApiInterface,
    script: Arc<Mutex<Script>>,
}

impl ScriptedProvider {
    /// A provider for `interface` without scripted calls.
    pub fn new(interface: ApiInterface) -> Self {
        Self {
            interface,
            script: Arc::default(),
        }
    }

    /// Adds the script of the next call that has none yet.
    pub fn push_call(&self, steps: Vec<Step>) {
        self.lock().calls.push_back(steps);
    }

    /// The requests of every call so far, in order.
    pub fn requests(&self) -> Vec<ModelRequest> {
        self.lock().requests.clone()
    }

    /// How many scripted calls have not been made yet.
    pub fn remaining_calls(&self) -> usize {
        self.lock().calls.len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Script> {
        self.script.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Provider for ScriptedProvider {
    fn interface(&self) -> ApiInterface {
        self.interface
    }

    fn stream(&self, _target: &Target, request: ModelRequest) -> EventStream {
        let steps = {
            let mut script = self.lock();
            script.requests.push(request);
            script.calls.pop_front()
        };
        // A call without a script fails, so that a test that makes more calls
        // than it expects notices.
        let steps: VecDeque<Step> = steps.map_or_else(
            || {
                VecDeque::from([Step::Error(ProviderError::Permanent(
                    mannie_types::run::RunFailure::Internal,
                ))])
            },
            VecDeque::from,
        );
        Box::pin(futures_util::stream::unfold(
            steps,
            |mut steps| async move {
                loop {
                    match steps.pop_front()? {
                        Step::Event(event) => return Some((Ok(event), steps)),
                        Step::Error(error) => {
                            steps.clear();
                            return Some((Err(error), steps));
                        }
                        Step::Delay(duration) => tokio::time::sleep(duration).await,
                        Step::Wait(ready) => ready.notified().await,
                        Step::Hang => std::future::pending::<()>().await,
                    }
                }
            },
        ))
    }
}

/// A response that the fixture server sends.
#[derive(Debug, Clone)]
pub struct FixtureResponse {
    /// The HTTP status.
    pub status: u16,
    /// Headers besides the framing headers, which the server sets itself.
    pub headers: Vec<(String, String)>,
    /// The body, sent as HTTP chunks.
    pub body: Vec<u8>,
    /// How the body is cut into chunks.
    pub chunking: Chunking,
    /// A pause before each chunk.
    pub chunk_delay: Duration,
    /// Whether the server stops sending before the end of the body and keeps
    /// the connection open, like a server that hangs.
    pub stall: bool,
}

impl FixtureResponse {
    /// An event stream with status 200.
    pub fn event_stream(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type".to_owned(), "text/event-stream".to_owned())],
            body: body.into(),
            chunking: Chunking::Whole,
            chunk_delay: Duration::ZERO,
            stall: false,
        }
    }

    /// A JSON document with the given status.
    pub fn json(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body: body.into(),
            chunking: Chunking::Whole,
            chunk_delay: Duration::ZERO,
            stall: false,
        }
    }
}

/// How the fixture server cuts a body into chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chunking {
    /// One chunk.
    Whole,
    /// Chunks of random sizes between 1 and `max` bytes, from a seed, so a
    /// failing run can be repeated.
    Random {
        /// The seed.
        seed: u64,
        /// The largest chunk.
        max: usize,
    },
}

/// A request that the fixture server received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedRequest {
    /// The method, such as `POST`.
    pub method: String,
    /// The path with its query.
    pub path: String,
    /// The headers, with lower-case names.
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: Vec<u8>,
}

impl RecordedRequest {
    /// The value of a header.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug, Default)]
struct FixtureState {
    responses: VecDeque<FixtureResponse>,
    requests: Vec<RecordedRequest>,
}

/// An HTTP/1.1 server on the loopback interface that answers each request
/// with the next queued response, so that adapters run against the bytes a
/// real server sent.
#[derive(Debug)]
pub struct FixtureServer {
    address: SocketAddr,
    state: Arc<Mutex<FixtureState>>,
    task: JoinHandle<()>,
}

impl FixtureServer {
    /// Starts the server on a free port.
    pub async fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let state: Arc<Mutex<FixtureState>> = Arc::default();
        let shared = Arc::clone(&state);
        let task = tokio::spawn(async move {
            while let Ok((connection, _peer)) = listener.accept().await {
                tokio::spawn(serve(connection, Arc::clone(&shared)));
            }
        });
        Ok(Self {
            address,
            state,
            task,
        })
    }

    /// The base URL of the server's API: `http://127.0.0.1:<port>/v1`.
    ///
    /// # Panics
    ///
    /// Never: a loopback address with a port is a valid URL.
    pub fn base_url(&self) -> Url {
        Url::parse(&format!("http://{}/v1", self.address))
            .expect("a loopback address with a port is a valid URL")
    }

    /// Queues the response to the next request.
    pub fn respond(&self, response: FixtureResponse) {
        lock(&self.state).responses.push_back(response);
    }

    /// The requests so far, in order.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        lock(&self.state).requests.clone()
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn lock(state: &Mutex<FixtureState>) -> std::sync::MutexGuard<'_, FixtureState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

async fn serve(mut connection: TcpStream, state: Arc<Mutex<FixtureState>>) {
    let Some(request) = read_request(&mut connection).await else {
        return;
    };
    let response = {
        let mut state = lock(&state);
        state.requests.push(request);
        state.responses.pop_front()
    };
    let response = response.unwrap_or_else(|| {
        FixtureResponse::json(500, r#"{"error":{"message":"no fixture queued"}}"#)
    });
    if write_response(&mut connection, &response).await.is_err() {
        // The client went away, as a stopped call does.
    }
}

async fn read_request(connection: &mut TcpStream) -> Option<RecordedRequest> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 8_192];
    let header_end = loop {
        let read = connection.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next()?.split(' ');
    let method = request_line.next()?.to_owned();
    let path = request_line.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let mut body = buffer[header_end..].to_vec();
    while body.len() < length {
        let read = connection.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    Some(RecordedRequest {
        method,
        path,
        headers,
        body,
    })
}

async fn write_response(
    connection: &mut TcpStream,
    response: &FixtureResponse,
) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {} Fixture\r\n", response.status);
    head.extend(
        response
            .headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}\r\n")),
    );
    head.push_str("transfer-encoding: chunked\r\nconnection: close\r\n\r\n");
    connection.write_all(head.as_bytes()).await?;
    connection.flush().await?;
    let chunks = cut(&response.body, response.chunking);
    let sent = if response.stall {
        chunks.len() / 2
    } else {
        chunks.len()
    };
    for chunk in &chunks[..sent] {
        if !response.chunk_delay.is_zero() {
            tokio::time::sleep(response.chunk_delay).await;
        }
        connection
            .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
            .await?;
        connection.write_all(chunk).await?;
        connection.write_all(b"\r\n").await?;
        connection.flush().await?;
    }
    if response.stall {
        std::future::pending::<()>().await;
    }
    connection.write_all(b"0\r\n\r\n").await?;
    connection.flush().await
}

/// Cuts a body into chunks.
fn cut(body: &[u8], chunking: Chunking) -> Vec<&[u8]> {
    match chunking {
        Chunking::Whole => {
            if body.is_empty() {
                Vec::new()
            } else {
                vec![body]
            }
        }
        Chunking::Random { seed, max } => {
            let mut state = seed | 1;
            let mut chunks = Vec::new();
            let mut rest = body;
            while !rest.is_empty() {
                // xorshift64: a small generator that repeats for a seed.
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let size = usize::try_from(state % u64::try_from(max.max(1)).unwrap_or(1))
                    .unwrap_or(0)
                    + 1;
                let (chunk, remaining) = rest.split_at(size.min(rest.len()));
                chunks.push(chunk);
                rest = remaining;
            }
            chunks
        }
    }
}

/// One request that the recording proxy forwarded, with the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exchange {
    /// The request as the adapter sent it.
    pub request: RecordedRequest,
    /// The server's status.
    pub status: u16,
    /// The server's headers, with lower-case names.
    pub headers: Vec<(String, String)>,
    /// The server's body, byte for byte.
    pub body: Vec<u8>,
}

/// A proxy on the loopback interface that forwards each request to a live
/// server and records the exchange, so that a live run can become a fixture
/// (`task provider:record`). The adapter talks plain HTTP to the proxy, which
/// talks to the server as the adapter would.
#[derive(Debug)]
pub struct RecordingProxy {
    address: SocketAddr,
    exchanges: Arc<Mutex<Vec<Exchange>>>,
    task: JoinHandle<()>,
}

impl RecordingProxy {
    /// Starts a proxy in front of the server at `upstream`, the server's base
    /// URL, such as `https://api.openai.com/v1`.
    pub async fn start(upstream: Url) -> std::io::Result<Self> {
        crate::transport::install_ring();
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(std::io::Error::other)?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let exchanges: Arc<Mutex<Vec<Exchange>>> = Arc::default();
        let shared = Arc::clone(&exchanges);
        let task = tokio::spawn(async move {
            while let Ok((connection, _peer)) = listener.accept().await {
                tokio::spawn(forward(
                    connection,
                    client.clone(),
                    upstream.clone(),
                    Arc::clone(&shared),
                ));
            }
        });
        Ok(Self {
            address,
            exchanges,
            task,
        })
    }

    /// The base URL to give the adapter: `http://127.0.0.1:<port>/v1`.
    ///
    /// # Panics
    ///
    /// Never: a loopback address with a port is a valid URL.
    pub fn base_url(&self) -> Url {
        Url::parse(&format!("http://{}/v1", self.address))
            .expect("a loopback address with a port is a valid URL")
    }

    /// The exchanges so far, in order.
    pub fn exchanges(&self) -> Vec<Exchange> {
        self.exchanges
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Drop for RecordingProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn forward(
    mut connection: TcpStream,
    client: reqwest::Client,
    upstream: Url,
    exchanges: Arc<Mutex<Vec<Exchange>>>,
) {
    let Some(request) = read_request(&mut connection).await else {
        return;
    };
    let rest = request.path.strip_prefix("/v1").unwrap_or(&request.path);
    let target = format!("{}{rest}", upstream.as_str().trim_end_matches('/'));
    let method =
        reqwest::Method::from_bytes(request.method.as_bytes()).unwrap_or(reqwest::Method::GET);
    let mut outgoing = client.request(method, target).body(request.body.clone());
    for name in ["authorization", "content-type", "accept"] {
        if let Some(value) = request.header(name) {
            outgoing = outgoing.header(name, value);
        }
    }
    let (status, headers, body) = match outgoing.send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            let headers: Vec<(String, String)> = response
                .headers()
                .iter()
                .filter_map(|(name, value)| {
                    Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned()))
                })
                .collect();
            let body = response
                .bytes()
                .await
                .map(|bytes| bytes.to_vec())
                .unwrap_or_default();
            (status, headers, body)
        }
        Err(error) => (
            502,
            Vec::new(),
            format!(r#"{{"error":{{"message":"{error}"}}}}"#).into_bytes(),
        ),
    };
    let kept: Vec<(String, String)> = headers
        .iter()
        .filter(|(name, _)| {
            ["content-type", "retry-after", "retry-after-ms"].contains(&name.as_str())
        })
        .cloned()
        .collect();
    let response = FixtureResponse {
        status,
        headers: kept.clone(),
        body: body.clone(),
        chunking: Chunking::Whole,
        chunk_delay: Duration::ZERO,
        stall: false,
    };
    exchanges
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Exchange {
            request,
            status,
            headers: kept,
            body,
        });
    if write_response(&mut connection, &response).await.is_err() {
        // The adapter went away.
    }
}
