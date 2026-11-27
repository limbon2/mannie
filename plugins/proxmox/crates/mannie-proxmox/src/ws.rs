//! Zero-unsafe pure-Rust RFC 6455 binary WebSocket client over any async TLS
//! stream (`tokio_rustls::client::TlsStream<TcpStream>`) using `ring` for
//! random masking keys and SHA-1 `Sec-WebSocket-Accept` verification (ADR-0080).

use std::sync::Arc;

use base64::Engine as _;
use ring::digest::{SHA1_FOR_LEGACY_USE_ONLY, digest};
use ring::rand::{SecureRandom as _, SystemRandom};
use tokio::io::{
    AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, ReadHalf, WriteHalf,
};
use tokio::sync::Mutex;

/// RFC 6455 §1.3 static GUID concatenated with `Sec-WebSocket-Key`.
const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Maximum HTTP 101 upgrade response header size (16 KiB).
const MAX_HANDSHAKE_HEADER_BYTES: usize = 16 * 1024;

/// Maximum length of a single gateway diagnostic header included in handshake errors.
const MAX_DIAGNOSTIC_HEADER_CHARS: usize = 80;

/// Maximum length of a plain-text body snippet included in non-101 handshake errors.
const MAX_DIAGNOSTIC_BODY_CHARS: usize = 160;

/// Maximum incoming WebSocket frame or assembled message size (16 MiB).
pub(crate) const MAX_WS_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// HTTP/1.1 WebSocket upgrade parameters for [`WsByteStream::connect`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct WsHandshake<'a> {
    pub(crate) host_header: &'a str,
    pub(crate) origin: &'a str,
    pub(crate) user_agent: &'a str,
    pub(crate) path_and_query: &'a str,
    pub(crate) cookie_header: &'a str,
}

/// Errors returned by the RFC 6455 client transport.
#[derive(Debug, thiserror::Error)]
pub(crate) enum WsError {
    /// Underlying stream I/O error.
    #[error("WebSocket I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Server rejected or malformed the HTTP 101 WebSocket handshake.
    #[error("WebSocket handshake failed: {0}")]
    Handshake(String),
    /// Server violated RFC 6455 framing or size limits.
    #[error("WebSocket protocol error: {0}")]
    Protocol(String),
    /// Random number generator failed.
    #[error("system random generator failed")]
    Random,
}

/// Split binary WebSocket read and write halves returned by [`WsByteStream::into_split`].
pub(crate) type SplitWs<S> = (
    WsByteReader<ReadHalf<S>, WriteHalf<S>>,
    Arc<Mutex<WsByteWriter<WriteHalf<S>>>>,
);

/// Binary WebSocket client with buffered byte-stream adapter for RFB 3.8.
#[derive(Debug)]
pub(crate) struct WsByteStream<S> {
    stream: S,
    rng: SystemRandom,
    /// Unparsed raw TCP/TLS bytes already read past HTTP headers.
    raw_buf: Vec<u8>,
    /// Decoded RFB application bytes waiting for `read_exact`.
    app_buf: Vec<u8>,
    app_pos: usize,
    closed: bool,
}

impl<S: AsyncRead + AsyncWrite + Unpin> WsByteStream<S> {
    /// Performs the HTTP/1.1 WebSocket upgrade handshake with
    /// `Sec-WebSocket-Protocol: binary`, `Origin`, `User-Agent`, and `Cookie`,
    /// verifying status 101 and `Sec-WebSocket-Accept`.
    pub(crate) async fn connect(
        mut stream: S,
        handshake: WsHandshake<'_>,
    ) -> Result<Self, WsError> {
        let rng = SystemRandom::new();
        let mut nonce = [0u8; 16];
        rng.fill(&mut nonce).map_err(|_err| WsError::Random)?;

        let ws_key = base64::engine::general_purpose::STANDARD.encode(nonce);
        let expected_accept = compute_accept_key(&ws_key);

        let request = format!(
            "GET {} HTTP/1.1\r\n\
             Host: {}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Origin: {}\r\n\
             User-Agent: {}\r\n\
             Pragma: no-cache\r\n\
             Cache-Control: no-cache\r\n\
             Sec-WebSocket-Key: {ws_key}\r\n\
             Sec-WebSocket-Version: 13\r\n\
             Sec-WebSocket-Protocol: binary\r\n\
             Cookie: {}\r\n\
             \r\n",
            handshake.path_and_query,
            handshake.host_header,
            handshake.origin,
            handshake.user_agent,
            handshake.cookie_header,
        );
        stream.write_all(request.as_bytes()).await?;
        stream.flush().await?;

        let mut header_buf = Vec::with_capacity(1024);
        let mut chunk = [0u8; 1024];
        let header_end = loop {
            let n = stream.read(&mut chunk).await?;
            if n == 0 {
                return Err(WsError::Handshake(
                    "connection closed before HTTP 101 headers".to_owned(),
                ));
            }

            header_buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = find_header_end(&header_buf) {
                break pos;
            }

            if header_buf.len() > MAX_HANDSHAKE_HEADER_BYTES {
                return Err(WsError::Handshake(
                    "HTTP upgrade response headers exceeded limit".to_owned(),
                ));
            }
        };

        let header_text = std::str::from_utf8(&header_buf[..header_end])
            .map_err(|_err| WsError::Handshake("non-UTF-8 HTTP response headers".to_owned()))?;
        let leftover = &header_buf[header_end + 4..];
        verify_handshake_response(header_text, leftover, &expected_accept)?;

        Ok(Self {
            stream,
            rng,
            raw_buf: leftover.to_vec(),
            app_buf: Vec::with_capacity(8192),
            app_pos: 0,
            closed: false,
        })
    }

    /// Reads exactly `buf.len()` RFB payload bytes from incoming binary frames.
    pub(crate) async fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), WsError> {
        let mut offset = 0;
        while offset < buf.len() {
            let available = self.app_buf.len().saturating_sub(self.app_pos);
            if available > 0 {
                let take = available.min(buf.len() - offset);
                buf[offset..offset + take]
                    .copy_from_slice(&self.app_buf[self.app_pos..self.app_pos + take]);
                self.app_pos += take;
                offset += take;
                if self.app_pos == self.app_buf.len() {
                    self.app_buf.clear();
                    self.app_pos = 0;
                }
                continue;
            }

            match self.read_next_message().await? {
                Some(payload) => {
                    if !payload.is_empty() {
                        self.app_buf = payload;
                        self.app_pos = 0;
                    }
                }
                None => {
                    return Err(WsError::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "WebSocket closed by server",
                    )));
                }
            }
        }
        Ok(())
    }

    /// Writes `data` as one masked client-to-server binary WebSocket frame.
    pub(crate) async fn write_all(&mut self, data: &[u8]) -> Result<(), WsError> {
        self.write_frame(0x02, data).await
    }

    /// Splits the WebSocket byte stream into full-duplex read and write halves
    /// so concurrent input writes never cancel an in-progress frame read.
    pub(crate) fn into_split(self) -> SplitWs<S> {
        let (read_half, write_half) = tokio::io::split(self.stream);
        let writer = Arc::new(Mutex::new(WsByteWriter {
            stream: write_half,
            rng: self.rng,
            closed: self.closed,
        }));
        let reader = WsByteReader {
            stream: read_half,
            writer: Arc::clone(&writer),
            raw_buf: self.raw_buf,
            app_buf: self.app_buf,
            app_pos: self.app_pos,
        };
        (reader, writer)
    }

    /// Reads the next complete binary message, transparently responding to
    /// Ping frames and handling RFC 6455 fragmentation.
    async fn read_next_message(&mut self) -> Result<Option<Vec<u8>>, WsError> {
        let mut message = Vec::new();
        let mut expecting_continuation = false;

        loop {
            let mut hdr = [0u8; 2];
            self.read_raw_exact(&mut hdr).await?;

            let fin = (hdr[0] & 0x80) != 0;
            let rsv = hdr[0] & 0x70;
            let opcode = hdr[0] & 0x0f;
            let masked = (hdr[1] & 0x80) != 0;
            let len7 = hdr[1] & 0x7f;

            if rsv != 0 {
                return Err(WsError::Protocol(
                    "unexpected reserved WebSocket bits set".to_owned(),
                ));
            }

            if masked {
                return Err(WsError::Protocol(
                    "server sent a masked WebSocket frame".to_owned(),
                ));
            }

            let payload_len = self.read_frame_length(len7).await?;
            if payload_len > MAX_WS_FRAME_BYTES {
                return Err(WsError::Protocol(
                    "incoming WebSocket frame exceeds size limit".to_owned(),
                ));
            }

            let mut payload = vec![0u8; payload_len];
            if payload_len > 0 {
                self.read_raw_exact(&mut payload).await?;
            }

            match opcode {
                // Close
                0x08 => {
                    if !self.closed {
                        self.closed = true;
                        let reply = if payload.len() >= 2 {
                            &payload[..2]
                        } else {
                            &1000u16.to_be_bytes()
                        };
                        drop(self.write_frame(0x08, reply).await);
                    }
                    return Ok(None);
                }
                // Ping -> reply with Pong carrying same payload (max 125 bytes per RFC)
                0x09 => {
                    if !fin || payload.len() > 125 {
                        return Err(WsError::Protocol("invalid WebSocket Ping frame".to_owned()));
                    }
                    self.write_frame(0x0a, &payload).await?;
                }
                // Pong -> ignore
                0x0a => {
                    if !fin || payload.len() > 125 {
                        return Err(WsError::Protocol("invalid WebSocket Pong frame".to_owned()));
                    }
                }
                // Binary (or text, tolerated if non-fragmented start? RFB uses binary only)
                0x02 => {
                    if expecting_continuation {
                        return Err(WsError::Protocol(
                            "unexpected new data frame during fragmented message".to_owned(),
                        ));
                    }

                    if fin {
                        return Ok(Some(payload));
                    }

                    message.extend_from_slice(&payload);
                    expecting_continuation = true;
                }
                // Continuation
                0x00 => {
                    if !expecting_continuation {
                        return Err(WsError::Protocol(
                            "unexpected WebSocket continuation frame".to_owned(),
                        ));
                    }

                    if message.len().saturating_add(payload.len()) > MAX_WS_FRAME_BYTES {
                        return Err(WsError::Protocol(
                            "fragmented WebSocket message exceeds size limit".to_owned(),
                        ));
                    }

                    message.extend_from_slice(&payload);
                    if fin {
                        return Ok(Some(message));
                    }
                }
                other => {
                    return Err(WsError::Protocol(format!(
                        "unsupported WebSocket opcode 0x{other:x}"
                    )));
                }
            }
        }
    }

    async fn read_frame_length(&mut self, len7: u8) -> Result<usize, WsError> {
        match len7 {
            0..=125 => Ok(usize::from(len7)),
            126 => {
                let mut buf = [0u8; 2];
                self.read_raw_exact(&mut buf).await?;
                Ok(usize::from(u16::from_be_bytes(buf)))
            }
            127 => {
                let mut buf = [0u8; 8];
                self.read_raw_exact(&mut buf).await?;
                let len = u64::from_be_bytes(buf);
                usize::try_from(len)
                    .map_err(|_err| WsError::Protocol("frame length overflows usize".to_owned()))
            }
            _ => Err(WsError::Protocol("invalid frame length marker".to_owned())),
        }
    }

    async fn read_raw_exact(&mut self, out: &mut [u8]) -> Result<(), WsError> {
        let mut offset = 0;
        if !self.raw_buf.is_empty() {
            let take = self.raw_buf.len().min(out.len());
            out[..take].copy_from_slice(&self.raw_buf[..take]);
            self.raw_buf.drain(..take);
            offset += take;
        }

        if offset < out.len() {
            self.stream.read_exact(&mut out[offset..]).await?;
        }

        Ok(())
    }

    async fn write_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), WsError> {
        let mut mask = [0u8; 4];
        self.rng.fill(&mut mask).map_err(|_err| WsError::Random)?;

        let mut frame = Vec::with_capacity(14 + payload.len());
        frame.push(0x80 | (opcode & 0x0f));

        let len = payload.len();
        if len < 126 {
            #[expect(clippy::cast_possible_truncation, reason = "len < 126 fits in u8")]
            frame.push(0x80 | (len as u8));
        } else if let Ok(len16) = u16::try_from(len) {
            frame.push(0x80 | 0x7e);
            frame.extend_from_slice(&len16.to_be_bytes());
        } else {
            let len64 = u64::try_from(len)
                .map_err(|_err| WsError::Protocol("frame length exceeds u64".to_owned()))?;
            frame.push(0x80 | 0x7f);
            frame.extend_from_slice(&len64.to_be_bytes());
        }

        frame.extend_from_slice(&mask);
        for (index, &byte) in payload.iter().enumerate() {
            frame.push(byte ^ mask[index & 3]);
        }

        self.stream.write_all(&frame).await?;
        self.stream.flush().await?;
        Ok(())
    }
}

/// Write half of a split binary WebSocket connection.
#[derive(Debug)]
pub(crate) struct WsByteWriter<W> {
    stream: W,
    rng: SystemRandom,
    closed: bool,
}

impl<W: AsyncWrite + Unpin> WsByteWriter<W> {
    /// Writes `data` as one masked client-to-server binary frame.
    pub(crate) async fn write_all(&mut self, data: &[u8]) -> Result<(), WsError> {
        self.write_frame(0x02, data).await
    }

    /// Sends a polite Close frame and shuts down the write half.
    pub(crate) async fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            drop(self.write_frame(0x08, &1000u16.to_be_bytes()).await);
            drop(self.stream.shutdown().await);
        }
    }

    async fn write_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), WsError> {
        let mut mask = [0u8; 4];
        self.rng.fill(&mut mask).map_err(|_err| WsError::Random)?;

        let mut frame = Vec::with_capacity(14 + payload.len());
        frame.push(0x80 | (opcode & 0x0f));

        let len = payload.len();
        if len < 126 {
            #[expect(clippy::cast_possible_truncation, reason = "len < 126 fits in u8")]
            frame.push(0x80 | (len as u8));
        } else if let Ok(len16) = u16::try_from(len) {
            frame.push(0x80 | 0x7e);
            frame.extend_from_slice(&len16.to_be_bytes());
        } else {
            let len64 = u64::try_from(len)
                .map_err(|_err| WsError::Protocol("frame length exceeds u64".to_owned()))?;
            frame.push(0x80 | 0x7f);
            frame.extend_from_slice(&len64.to_be_bytes());
        }

        frame.extend_from_slice(&mask);
        for (index, &byte) in payload.iter().enumerate() {
            frame.push(byte ^ mask[index & 3]);
        }

        self.stream.write_all(&frame).await?;
        self.stream.flush().await?;
        Ok(())
    }
}

/// Read half of a split binary WebSocket connection.
#[derive(Debug)]
pub(crate) struct WsByteReader<R, W> {
    stream: R,
    writer: Arc<Mutex<WsByteWriter<W>>>,
    raw_buf: Vec<u8>,
    app_buf: Vec<u8>,
    app_pos: usize,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> WsByteReader<R, W> {
    /// Reads exactly `buf.len()` RFB payload bytes from incoming binary frames.
    pub(crate) async fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), WsError> {
        let mut offset = 0;
        while offset < buf.len() {
            let available = self.app_buf.len().saturating_sub(self.app_pos);
            if available > 0 {
                let take = available.min(buf.len() - offset);
                buf[offset..offset + take]
                    .copy_from_slice(&self.app_buf[self.app_pos..self.app_pos + take]);
                self.app_pos += take;
                offset += take;
                if self.app_pos == self.app_buf.len() {
                    self.app_buf.clear();
                    self.app_pos = 0;
                }
                continue;
            }

            match self.read_next_message().await? {
                Some(payload) => {
                    if !payload.is_empty() {
                        self.app_buf = payload;
                        self.app_pos = 0;
                    }
                }
                None => {
                    return Err(WsError::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "WebSocket closed by server",
                    )));
                }
            }
        }
        Ok(())
    }

    async fn read_next_message(&mut self) -> Result<Option<Vec<u8>>, WsError> {
        let mut message = Vec::new();
        let mut expecting_continuation = false;

        loop {
            let mut hdr = [0u8; 2];
            self.read_raw_exact(&mut hdr).await?;

            let fin = (hdr[0] & 0x80) != 0;
            let rsv = hdr[0] & 0x70;
            let opcode = hdr[0] & 0x0f;
            let masked = (hdr[1] & 0x80) != 0;
            let len7 = hdr[1] & 0x7f;

            if rsv != 0 {
                return Err(WsError::Protocol(
                    "unexpected reserved WebSocket bits set".to_owned(),
                ));
            }

            if masked {
                return Err(WsError::Protocol(
                    "server sent a masked WebSocket frame".to_owned(),
                ));
            }

            let payload_len = self.read_frame_length(len7).await?;
            if payload_len > MAX_WS_FRAME_BYTES {
                return Err(WsError::Protocol(
                    "incoming WebSocket frame exceeds size limit".to_owned(),
                ));
            }

            let mut payload = vec![0u8; payload_len];
            if payload_len > 0 {
                self.read_raw_exact(&mut payload).await?;
            }

            match opcode {
                0x08 => {
                    let reply = if payload.len() >= 2 {
                        &payload[..2]
                    } else {
                        &1000u16.to_be_bytes()
                    };
                    let mut writer = self.writer.lock().await;
                    if !writer.closed {
                        writer.closed = true;
                        drop(writer.write_frame(0x08, reply).await);
                    }
                    return Ok(None);
                }
                0x09 => {
                    if !fin || payload.len() > 125 {
                        return Err(WsError::Protocol("invalid WebSocket Ping frame".to_owned()));
                    }
                    let mut writer = self.writer.lock().await;
                    writer.write_frame(0x0a, &payload).await?;
                }
                0x0a => {
                    if !fin || payload.len() > 125 {
                        return Err(WsError::Protocol("invalid WebSocket Pong frame".to_owned()));
                    }
                }
                0x02 => {
                    if expecting_continuation {
                        return Err(WsError::Protocol(
                            "unexpected new data frame during fragmented message".to_owned(),
                        ));
                    }

                    if fin {
                        return Ok(Some(payload));
                    }

                    message.extend_from_slice(&payload);
                    expecting_continuation = true;
                }
                0x00 => {
                    if !expecting_continuation {
                        return Err(WsError::Protocol(
                            "unexpected WebSocket continuation frame".to_owned(),
                        ));
                    }

                    if message.len().saturating_add(payload.len()) > MAX_WS_FRAME_BYTES {
                        return Err(WsError::Protocol(
                            "fragmented WebSocket message exceeds size limit".to_owned(),
                        ));
                    }

                    message.extend_from_slice(&payload);
                    if fin {
                        return Ok(Some(message));
                    }
                }
                other => {
                    return Err(WsError::Protocol(format!(
                        "unsupported WebSocket opcode 0x{other:x}"
                    )));
                }
            }
        }
    }

    async fn read_frame_length(&mut self, len7: u8) -> Result<usize, WsError> {
        match len7 {
            0..=125 => Ok(usize::from(len7)),
            126 => {
                let mut buf = [0u8; 2];
                self.read_raw_exact(&mut buf).await?;
                Ok(usize::from(u16::from_be_bytes(buf)))
            }
            127 => {
                let mut buf = [0u8; 8];
                self.read_raw_exact(&mut buf).await?;
                let len = u64::from_be_bytes(buf);
                usize::try_from(len)
                    .map_err(|_err| WsError::Protocol("frame length overflows usize".to_owned()))
            }
            _ => Err(WsError::Protocol("invalid frame length marker".to_owned())),
        }
    }

    async fn read_raw_exact(&mut self, out: &mut [u8]) -> Result<(), WsError> {
        let mut offset = 0;
        if !self.raw_buf.is_empty() {
            let take = self.raw_buf.len().min(out.len());
            out[..take].copy_from_slice(&self.raw_buf[..take]);
            self.raw_buf.drain(..take);
            offset += take;
        }

        if offset < out.len() {
            self.stream.read_exact(&mut out[offset..]).await?;
        }

        Ok(())
    }
}

/// Computes RFC 6455 `Sec-WebSocket-Accept` from `Sec-WebSocket-Key`.
pub(crate) fn compute_accept_key(key: &str) -> String {
    let mut combined = String::with_capacity(key.len() + WS_GUID.len());
    combined.push_str(key);
    combined.push_str(WS_GUID);
    let sha1 = digest(&SHA1_FOR_LEGACY_USE_ONLY, combined.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(sha1.as_ref())
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn verify_handshake_response(
    headers: &str,
    body_preview: &[u8],
    expected_accept: &str,
) -> Result<(), WsError> {
    let mut lines = headers.split("\r\n");
    let status_line = lines
        .next()
        .map(str::trim)
        .ok_or_else(|| WsError::Handshake("empty HTTP response".to_owned()))?;
    let is_101 = status_line.starts_with("HTTP/1.1 101") || status_line.starts_with("HTTP/1.0 101");

    if !is_101 {
        let detail = format_unexpected_status(status_line, lines, body_preview);
        return Err(WsError::Handshake(detail));
    }

    let mut upgrade_ok = false;
    let mut connection_ok = false;
    let mut accept_ok = false;

    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        let is_upgrade_ws =
            name.eq_ignore_ascii_case("upgrade") && value.eq_ignore_ascii_case("websocket");
        let is_connection_upgrade = name.eq_ignore_ascii_case("connection")
            && value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"));
        let is_accept_match =
            name.eq_ignore_ascii_case("sec-websocket-accept") && value == expected_accept;

        if is_upgrade_ws {
            upgrade_ok = true;
        } else if is_connection_upgrade {
            connection_ok = true;
        } else if is_accept_match {
            accept_ok = true;
        }
    }

    let handshake_valid = upgrade_ok && connection_ok && accept_ok;

    if !handshake_valid {
        return Err(WsError::Handshake(
            "server HTTP 101 response missing valid WebSocket upgrade headers".to_owned(),
        ));
    }

    Ok(())
}

/// Formats a concise, sanitized diagnostic message for a non-101 handshake
/// response, keeping only safe gateway metadata (`server`, `via`) and a short
/// plain-text excerpt of any buffered body.
fn format_unexpected_status<'a>(
    status_line: &str,
    header_lines: impl Iterator<Item = &'a str>,
    body_preview: &[u8],
) -> String {
    let mut hints = Vec::new();

    for line in header_lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let label = if name.eq_ignore_ascii_case("server") {
            "server"
        } else if name.eq_ignore_ascii_case("via") {
            "via"
        } else {
            continue;
        };
        let clean = sanitize_inline(value, MAX_DIAGNOSTIC_HEADER_CHARS);

        if !clean.is_empty() {
            hints.push(format!("{label}: {clean}"));
        }
    }

    let mut out = format!("server returned unexpected status line: {status_line}");

    if !hints.is_empty() {
        out.push_str(" (");
        out.push_str(&hints.join(", "));
        out.push(')');
    }

    let snippet = extract_body_snippet(body_preview, MAX_DIAGNOSTIC_BODY_CHARS);

    if !snippet.is_empty() {
        out.push_str(": ");
        out.push_str(&snippet);
    }

    out
}

fn sanitize_inline(raw: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut pending_space = false;
    let mut count = 0usize;

    for ch in raw.chars() {
        if ch.is_control() || ch.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }

        if pending_space {
            if count >= max_chars {
                break;
            }

            out.push(' ');
            count += 1;
            pending_space = false;
        }

        if count >= max_chars {
            break;
        }

        out.push(ch);
        count += 1;
    }

    out
}

fn extract_body_snippet(bytes: &[u8], max_chars: usize) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    let text = String::from_utf8_lossy(bytes);
    let mut stripped = String::with_capacity(text.len().min(max_chars * 2));
    let mut in_tag = false;

    for ch in text.chars() {
        if ch == '<' {
            in_tag = true;
            stripped.push(' ');
        } else if ch == '>' {
            in_tag = false;
            stripped.push(' ');
        } else if !in_tag {
            stripped.push(ch);
        }
    }

    sanitize_inline(&stripped, max_chars)
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::{WsByteStream, WsHandshake, compute_accept_key, verify_handshake_response};

    #[test]
    fn rfc6455_accept_key_matches_spec_example() {
        // RFC 6455 §1.3 official test vector.
        assert_eq!(
            compute_accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn non_101_response_includes_gateway_headers_and_body_without_cookies() {
        let headers = "HTTP/1.0 502 Bad Gateway\r\n\
             Server: haproxy\r\n\
             Via: 1.1 edge-gw\r\n\
             Set-Cookie: SECRET_SESSION=should_not_appear\r\n\
             Content-Type: text/html";
        let body = b"<html><body><h1>502 Bad Gateway</h1>\nThe server returned an invalid or incomplete response.\n</body></html>";
        let err = verify_handshake_response(headers, body, "unused").expect_err("expected 502");
        let text = err.to_string();

        assert!(text.contains("HTTP/1.0 502 Bad Gateway"), "got: {text}");
        assert!(text.contains("server: haproxy"), "got: {text}");
        assert!(text.contains("via: 1.1 edge-gw"), "got: {text}");
        assert!(
            text.contains("The server returned an invalid or incomplete response."),
            "got: {text}"
        );
        assert!(
            !text.contains("SECRET_SESSION"),
            "must not echo Set-Cookie: {text}"
        );
    }

    #[tokio::test]
    async fn connect_sends_browser_compatible_upgrade_headers() {
        let (client_io, mut server_io) = tokio::io::duplex(4096);
        let server_task = tokio::spawn(async move {
            let mut raw = Vec::new();
            let mut buf = [0u8; 512];
            loop {
                let n = server_io.read(&mut buf).await.unwrap();
                assert!(n > 0, "unexpected EOF reading WS request");
                raw.extend_from_slice(&buf[..n]);
                if raw.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }

            let req = String::from_utf8(raw).unwrap();
            let key = req
                .lines()
                .find_map(|line| line.strip_prefix("Sec-WebSocket-Key: "))
                .expect("Sec-WebSocket-Key header")
                .trim()
                .to_owned();
            let accept = compute_accept_key(&key);
            let reply = format!(
                "HTTP/1.1 101 Switching Protocols\r\n\
                 Upgrade: websocket\r\n\
                 Connection: Upgrade\r\n\
                 Sec-WebSocket-Accept: {accept}\r\n\
                 Sec-WebSocket-Protocol: binary\r\n\
                 \r\n"
            );
            server_io.write_all(reply.as_bytes()).await.unwrap();
            server_io.flush().await.unwrap();
            req
        });

        let handshake = WsHandshake {
            host_header: "pve.work.example",
            origin: "https://pve.work.example",
            user_agent: "Mannie/test",
            path_and_query: "/api2/json/nodes/pve1/qemu/768/vncwebsocket?port=5900&vncticket=tkt",
            cookie_header: "PVEAuthCookie=abc; SERVERID=pve1",
        };
        let _ws = WsByteStream::connect(client_io, handshake)
            .await
            .expect("WS upgrade succeeds");

        let req = server_task.await.unwrap();
        assert!(req.contains("Host: pve.work.example\r\n"), "got:\n{req}");
        assert!(
            req.contains("Origin: https://pve.work.example\r\n"),
            "got:\n{req}"
        );
        assert!(req.contains("User-Agent: Mannie/test\r\n"), "got:\n{req}");
        assert!(req.contains("Pragma: no-cache\r\n"), "got:\n{req}");
        assert!(req.contains("Cache-Control: no-cache\r\n"), "got:\n{req}");
        assert!(
            req.contains("Sec-WebSocket-Protocol: binary\r\n"),
            "got:\n{req}"
        );
        assert!(
            req.contains("Cookie: PVEAuthCookie=abc; SERVERID=pve1\r\n"),
            "got:\n{req}"
        );
    }
}
