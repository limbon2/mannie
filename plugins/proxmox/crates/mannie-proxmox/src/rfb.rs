//! Zero-unsafe pure-Rust RFB 3.8 client (RFC 6143) over `WsByteStream`:
//! - Handshake & RFC 6143 `SecurityType` 2 VNC Authentication (56-bit DES-ECB
//!   with bit-reversed key bytes, tested against NIST FIPS 46-3 & RFC 6143).
//! - 32bpp RGBA8888 framebuffer (`Raw`, `CopyRect`, `Zlib`, `DesktopSize`).
//! - Strict bounds on dimensions, rectangle count, and zlib inflation (T-126).
//! - PNG encoding for live canvas patches and ADR-0044 tool screenshots.

use std::io::Cursor;
use std::sync::Arc;

use flate2::{Decompress, FlushDecompress, Status};
use mannie_plugin_sdk::contracts::{MAX_TOOL_IMAGE_BYTES, MAX_TOOL_IMAGE_SIDE};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncRead, AsyncWrite, ReadHalf, WriteHalf};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

use crate::model::ProxmoxError;
use crate::ws::{WsByteReader, WsByteStream, WsByteWriter, WsError};

/// Maximum framebuffer dimensions accepted from a VNC server (T-126).
pub(crate) const MAX_FB_WIDTH: u16 = 4096;
pub(crate) const MAX_FB_HEIGHT: u16 = 4096;

/// Maximum rectangles in a single `FramebufferUpdate` message.
const MAX_RECTS_PER_UPDATE: u16 = 4096;

/// Maximum compressed payload for one RFB Zlib rectangle (16 MiB).
const MAX_ZLIB_COMPRESSED_BYTES: usize = 16 * 1024 * 1024;

/// Maximum inflated bytes for one RFB Zlib rectangle (64 MiB = 4096*4096*4).
const MAX_ZLIB_INFLATED_BYTES: usize = 64 * 1024 * 1024;

/// Maximum `ServerCutText` or desktop name length skipped/read (256 KiB).
const MAX_SERVER_TEXT_BYTES: usize = 256 * 1024;

/// RFB encoding constants (RFC 6143 §7.7).
const ENCODING_RAW: i32 = 0;
const ENCODING_COPY_RECT: i32 = 1;
const ENCODING_ZLIB: i32 = 6;
const ENCODING_DESKTOP_SIZE: i32 = -223;

/// Axis-aligned rectangle inside the framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rect {
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) width: u16,
    pub(crate) height: u16,
}

impl Rect {
    /// Expands `self` to enclose `other`.
    pub(crate) fn union(self, other: Self) -> Self {
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = self
            .x
            .saturating_add(self.width)
            .max(other.x.saturating_add(other.width));
        let y1 = self
            .y
            .saturating_add(self.height)
            .max(other.y.saturating_add(other.height));
        Self {
            x: x0,
            y: y0,
            width: x1.saturating_sub(x0),
            height: y1.saturating_sub(y0),
        }
    }
}

/// In-memory RGBA8888 framebuffer maintained by the RFB client.
#[derive(Debug, Clone)]
pub(crate) struct Framebuffer {
    pub(crate) width: u16,
    pub(crate) height: u16,
    /// Tightly packed RGBA8888 pixels (`width * height * 4` bytes).
    pub(crate) rgba: Vec<u8>,
    /// Monotonic counter incremented on every framebuffer update or resize.
    pub(crate) seq: u64,
    /// SHA-256 digest of the current framebuffer pixels for change detection.
    pub(crate) hash: [u8; 32],
}

impl Framebuffer {
    /// Creates an empty opaque black framebuffer of `width x height`.
    pub(crate) fn new(width: u16, height: u16) -> Result<Self, ProxmoxError> {
        validate_dimensions(width, height)?;
        let pixel_count = usize::from(width) * usize::from(height);
        let mut rgba = vec![0u8; pixel_count * 4];
        for chunk in rgba.as_chunks_mut::<4>().0 {
            chunk[3] = 255;
        }
        let hash: [u8; 32] = Sha256::digest(&rgba).into();
        Ok(Self {
            width,
            height,
            rgba,
            seq: 0,
            hash,
        })
    }

    /// Resizes the framebuffer (for `DesktopSize` pseudo-encoding).
    pub(crate) fn resize(&mut self, width: u16, height: u16) -> Result<(), ProxmoxError> {
        let fresh = Self::new(width, height)?;
        self.width = fresh.width;
        self.height = fresh.height;
        self.rgba = fresh.rgba;
        self.seq = self.seq.saturating_add(1);
        self.hash = fresh.hash;
        Ok(())
    }

    /// Writes a Raw or inflated RGBA rectangle into the framebuffer.
    /// Forces alpha = 255 because RFB 32bpp pads the unused fourth byte with 0.
    pub(crate) fn apply_raw(&mut self, rect: Rect, pixels: &[u8]) -> Result<(), ProxmoxError> {
        self.check_rect(rect)?;
        let expected = usize::from(rect.width) * usize::from(rect.height) * 4;
        if pixels.len() != expected {
            return Err(ProxmoxError::VncFailed {
                detail: "RFB rectangle pixel length mismatch".to_owned(),
            });
        }

        let fb_stride = usize::from(self.width) * 4;
        let rect_stride = usize::from(rect.width) * 4;
        for row in 0..usize::from(rect.height) {
            let dst_y = usize::from(rect.y) + row;
            let dst_start = dst_y * fb_stride + usize::from(rect.x) * 4;
            let src_start = row * rect_stride;
            let dst_row = &mut self.rgba[dst_start..dst_start + rect_stride];
            let src_row = &pixels[src_start..src_start + rect_stride];
            dst_row.copy_from_slice(src_row);
            for px in dst_row.as_chunks_mut::<4>().0 {
                px[3] = 255;
            }
        }
        Ok(())
    }

    /// Copies a rectangle from `(src_x, src_y)` to `rect` safely even when overlapping.
    pub(crate) fn apply_copy_rect(
        &mut self,
        rect: Rect,
        src_x: u16,
        src_y: u16,
    ) -> Result<(), ProxmoxError> {
        self.check_rect(rect)?;
        let src_rect = Rect {
            x: src_x,
            y: src_y,
            width: rect.width,
            height: rect.height,
        };
        self.check_rect(src_rect)?;

        let fb_stride = usize::from(self.width) * 4;
        let rect_stride = usize::from(rect.width) * 4;
        let mut temp = vec![0u8; usize::from(rect.height) * rect_stride];
        for row in 0..usize::from(rect.height) {
            let sy = usize::from(src_y) + row;
            let s_start = sy * fb_stride + usize::from(src_x) * 4;
            let t_start = row * rect_stride;
            temp[t_start..t_start + rect_stride]
                .copy_from_slice(&self.rgba[s_start..s_start + rect_stride]);
        }

        for row in 0..usize::from(rect.height) {
            let dy = usize::from(rect.y) + row;
            let d_start = dy * fb_stride + usize::from(rect.x) * 4;
            let t_start = row * rect_stride;
            self.rgba[d_start..d_start + rect_stride]
                .copy_from_slice(&temp[t_start..t_start + rect_stride]);
        }
        Ok(())
    }

    /// Recomputes `hash` and increments `seq` after applying an update batch.
    pub(crate) fn commit_update(&mut self) {
        self.seq = self.seq.saturating_add(1);
        self.hash = Sha256::digest(&self.rgba).into();
    }

    /// Encodes a sub-rectangle as PNG bytes (for live VNC panel tiles).
    pub(crate) fn encode_png_rect(&self, rect: Rect) -> Result<Vec<u8>, ProxmoxError> {
        self.check_rect(rect)?;
        let fb_stride = usize::from(self.width) * 4;
        let rect_stride = usize::from(rect.width) * 4;
        let mut sub = Vec::with_capacity(usize::from(rect.height) * rect_stride);
        for row in 0..usize::from(rect.height) {
            let y = usize::from(rect.y) + row;
            let start = y * fb_stride + usize::from(rect.x) * 4;
            sub.extend_from_slice(&self.rgba[start..start + rect_stride]);
        }
        encode_rgba_png(u32::from(rect.width), u32::from(rect.height), &sub)
    }

    /// Encodes the full framebuffer as PNG bytes within `MAX_TOOL_IMAGE_SIDE`
    /// and `MAX_TOOL_IMAGE_BYTES` (ADR-0044), downscaling cleanly if needed.
    pub(crate) fn encode_tool_screenshot(&self) -> Result<(Vec<u8>, u16, u16), ProxmoxError> {
        let mut target_w = u32::from(self.width);
        let mut target_h = u32::from(self.height);

        let max_side = MAX_TOOL_IMAGE_SIDE;
        if target_w > max_side || target_h > max_side {
            let scale_num = max_side;
            let scale_den = target_w.max(target_h);
            target_w = (target_w * scale_num / scale_den).max(1);
            target_h = (target_h * scale_num / scale_den).max(1);
        }

        for _attempt in 0..4 {
            let pixels = if target_w == u32::from(self.width) && target_h == u32::from(self.height)
            {
                self.rgba.clone()
            } else {
                downscale_rgba(
                    &self.rgba,
                    usize::from(self.width),
                    usize::from(self.height),
                    target_w as usize,
                    target_h as usize,
                )
            };
            let encoded = encode_rgba_png(target_w, target_h, &pixels)?;
            if encoded.len() <= MAX_TOOL_IMAGE_BYTES {
                let w = u16::try_from(target_w).unwrap_or(self.width);
                let h = u16::try_from(target_h).unwrap_or(self.height);
                return Ok((encoded, w, h));
            }

            target_w = (target_w * 3 / 4).max(1);
            target_h = (target_h * 3 / 4).max(1);
        }

        Err(ProxmoxError::VncFailed {
            detail: "encoded screenshot exceeded tool image byte limit".to_owned(),
        })
    }

    fn check_rect(&self, rect: Rect) -> Result<(), ProxmoxError> {
        let x_end = u32::from(rect.x) + u32::from(rect.width);
        let y_end = u32::from(rect.y) + u32::from(rect.height);
        let in_bounds = x_end <= u32::from(self.width) && y_end <= u32::from(self.height);
        if !in_bounds {
            return Err(ProxmoxError::VncFailed {
                detail: "RFB rectangle coordinates exceed framebuffer bounds".to_owned(),
            });
        }
        Ok(())
    }
}

/// Outcome of reading one server-to-client RFB message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RfbEvent {
    /// One or more rectangles updated inside the framebuffer.
    Updated {
        /// Bounding box of changed pixels.
        dirty: Rect,
        /// `true` when `DesktopSize` resized the framebuffer.
        resized: bool,
    },
    /// Non-visual message (Bell, `ColourMap`, `ServerCutText`).
    Other,
}

/// Split read and write halves returned by [`RfbClient::into_split`].
pub(crate) type SplitRfb<S> = (
    RfbReader<ReadHalf<S>, WriteHalf<S>>,
    RfbWriter<WriteHalf<S>>,
);

/// Live RFB 3.8 connection state owning the WebSocket byte stream and persistent
/// zlib inflation context.
pub(crate) struct RfbClient<S> {
    ws: WsByteStream<S>,
    zlib: Decompress,
}

impl<S: std::fmt::Debug> std::fmt::Debug for RfbClient<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RfbClient").finish_non_exhaustive()
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> RfbClient<S> {
    /// Performs the RFB 3.8 handshake, authenticates via `SecurityType` 2 DES
    /// (or `SecurityType` 1 None if no password is needed and server offers only None),
    /// configures 32bpp RGBA8888 pixel format and encodings, and returns the
    /// initial `Framebuffer`.
    pub(crate) async fn handshake(
        mut ws: WsByteStream<S>,
        password: &Zeroizing<String>,
    ) -> Result<(Self, Framebuffer), ProxmoxError> {
        // 1. ProtocolVersion (12 bytes: "RFB 003.008\n")
        let mut version = [0u8; 12];
        ws.read_exact(&mut version).await.map_err(ws_err)?;
        if !version.starts_with(b"RFB 003.") {
            return Err(ProxmoxError::VncFailed {
                detail: "server did not speak RFB 3.x".to_owned(),
            });
        }
        ws.write_all(b"RFB 003.008\n").await.map_err(ws_err)?;

        // 2. Security types
        let mut count_buf = [0u8; 1];
        ws.read_exact(&mut count_buf).await.map_err(ws_err)?;
        let num_types = usize::from(count_buf[0]);
        if num_types == 0 {
            let reason = read_rfb_string(&mut ws).await?;
            return Err(ProxmoxError::VncFailed {
                detail: format!("VNC server rejected handshake: {reason}"),
            });
        }

        let mut types = vec![0u8; num_types];
        ws.read_exact(&mut types).await.map_err(ws_err)?;

        if types.contains(&2) {
            // SecurityType 2: VNC Authentication (DES-ECB)
            ws.write_all(&[2u8]).await.map_err(ws_err)?;
            let mut challenge = [0u8; 16];
            ws.read_exact(&mut challenge).await.map_err(ws_err)?;
            let response = vnc_des_encrypt_challenge(password.as_bytes(), &challenge);
            ws.write_all(&response).await.map_err(ws_err)?;
        } else if types.contains(&1) && password.is_empty() {
            // SecurityType 1: None
            ws.write_all(&[1u8]).await.map_err(ws_err)?;
        } else {
            return Err(ProxmoxError::VncFailed {
                detail: "VNC server offered no supported RFB security type".to_owned(),
            });
        }

        // 3. SecurityResult (u32: 0 = OK)
        let mut sec_result = [0u8; 4];
        ws.read_exact(&mut sec_result).await.map_err(ws_err)?;
        if u32::from_be_bytes(sec_result) != 0 {
            let reason = read_rfb_string(&mut ws)
                .await
                .unwrap_or_else(|_| "authentication rejected".to_owned());
            return Err(ProxmoxError::VncFailed {
                detail: format!("VNC authentication failed: {reason}"),
            });
        }

        // 4. ClientInit (shared = 1)
        ws.write_all(&[1u8]).await.map_err(ws_err)?;

        // 5. ServerInit: width(2) + height(2) + pixel_format(16) + name_len(4) + name
        let mut server_init = [0u8; 20];
        ws.read_exact(&mut server_init).await.map_err(ws_err)?;
        let width = u16::from_be_bytes([server_init[0], server_init[1]]);
        let height = u16::from_be_bytes([server_init[2], server_init[3]]);
        let _desktop_name = read_rfb_string(&mut ws).await?;

        let framebuffer = Framebuffer::new(width, height)?;

        // 6. SetPixelFormat: 32bpp RGBA8888 little-endian (R shift 0, G shift 8, B shift 16)
        let set_pixel_format: [u8; 20] = [
            0, // message-type = SetPixelFormat
            0, 0, 0,  // padding
            32, // bits-per-pixel
            24, // depth
            0,  // big-endian-flag = false
            1,  // true-colour-flag = true
            0, 255, // red-max = 255
            0, 255, // green-max = 255
            0, 255, // blue-max = 255
            0,   // red-shift
            8,   // green-shift
            16,  // blue-shift
            0, 0, 0, // padding
        ];
        ws.write_all(&set_pixel_format).await.map_err(ws_err)?;

        // 7. SetEncodings: CopyRect, Zlib, Raw, DesktopSize
        let encodings = [
            ENCODING_COPY_RECT,
            ENCODING_ZLIB,
            ENCODING_RAW,
            ENCODING_DESKTOP_SIZE,
        ];
        let mut set_enc = Vec::with_capacity(4 + encodings.len() * 4);
        set_enc.push(2); // message-type = SetEncodings
        set_enc.push(0); // padding
        let enc_count = u16::try_from(encodings.len()).unwrap_or(0);
        set_enc.extend_from_slice(&enc_count.to_be_bytes());
        for encoding in encodings {
            set_enc.extend_from_slice(&encoding.to_be_bytes());
        }
        ws.write_all(&set_enc).await.map_err(ws_err)?;

        let mut client = Self {
            ws,
            zlib: Decompress::new(true),
        };
        // Request the initial full framebuffer immediately.
        client
            .send_framebuffer_update_request(false, 0, 0, width, height)
            .await?;

        Ok((client, framebuffer))
    }

    /// Sends a `FramebufferUpdateRequest` (RFB message type 3).
    pub(crate) async fn send_framebuffer_update_request(
        &mut self,
        incremental: bool,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
    ) -> Result<(), ProxmoxError> {
        let mut msg = [0u8; 10];
        msg[0] = 3;
        msg[1] = u8::from(incremental);
        msg[2..4].copy_from_slice(&x.to_be_bytes());
        msg[4..6].copy_from_slice(&y.to_be_bytes());
        msg[6..8].copy_from_slice(&width.to_be_bytes());
        msg[8..10].copy_from_slice(&height.to_be_bytes());
        self.ws.write_all(&msg).await.map_err(ws_err)
    }

    /// Splits the authenticated RFB client into full-duplex read and write
    /// halves for the live VNC session loop.
    pub(crate) fn into_split(self) -> SplitRfb<S> {
        let (ws_reader, ws_writer) = self.ws.into_split();
        (
            RfbReader {
                ws: ws_reader,
                zlib: self.zlib,
            },
            RfbWriter { ws: ws_writer },
        )
    }
}

/// Write half of a split RFB 3.8 connection, clonable for concurrent input and
/// update requests serialized behind a Tokio mutex.
#[derive(Debug)]
pub(crate) struct RfbWriter<W> {
    ws: Arc<Mutex<WsByteWriter<W>>>,
}

impl<W> Clone for RfbWriter<W> {
    fn clone(&self) -> Self {
        Self {
            ws: Arc::clone(&self.ws),
        }
    }
}

impl<W: AsyncWrite + Unpin> RfbWriter<W> {
    /// Sends a `FramebufferUpdateRequest` (RFB message type 3).
    pub(crate) async fn send_framebuffer_update_request(
        &self,
        incremental: bool,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
    ) -> Result<(), ProxmoxError> {
        let mut msg = [0u8; 10];
        msg[0] = 3;
        msg[1] = u8::from(incremental);
        msg[2..4].copy_from_slice(&x.to_be_bytes());
        msg[4..6].copy_from_slice(&y.to_be_bytes());
        msg[6..8].copy_from_slice(&width.to_be_bytes());
        msg[8..10].copy_from_slice(&height.to_be_bytes());
        let mut guard = self.ws.lock().await;
        guard.write_all(&msg).await.map_err(ws_err)
    }

    /// Sends a `KeyEvent` (RFB message type 4).
    pub(crate) async fn send_key_event(&self, keysym: u32, down: bool) -> Result<(), ProxmoxError> {
        let mut msg = [0u8; 8];
        msg[0] = 4;
        msg[1] = u8::from(down);
        msg[4..8].copy_from_slice(&keysym.to_be_bytes());
        let mut guard = self.ws.lock().await;
        guard.write_all(&msg).await.map_err(ws_err)
    }

    /// Sends a `PointerEvent` (RFB message type 5).
    pub(crate) async fn send_pointer_event(
        &self,
        x: u16,
        y: u16,
        button_mask: u8,
    ) -> Result<(), ProxmoxError> {
        let mut msg = [0u8; 6];
        msg[0] = 5;
        msg[1] = button_mask;
        msg[2..4].copy_from_slice(&x.to_be_bytes());
        msg[4..6].copy_from_slice(&y.to_be_bytes());
        let mut guard = self.ws.lock().await;
        guard.write_all(&msg).await.map_err(ws_err)
    }

    /// Sends a polite WebSocket Close frame and shuts down the stream.
    pub(crate) async fn close(&self) {
        let mut guard = self.ws.lock().await;
        guard.close().await;
    }
}

/// Read half of a split RFB 3.8 connection.
pub(crate) struct RfbReader<R, W> {
    ws: WsByteReader<R, W>,
    zlib: Decompress,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> RfbReader<R, W> {
    /// Reads and applies the next RFB server message onto `fb`.
    pub(crate) async fn read_next_event(
        &mut self,
        fb: &mut Framebuffer,
    ) -> Result<RfbEvent, ProxmoxError> {
        let mut msg_type = [0u8; 1];
        self.ws.read_exact(&mut msg_type).await.map_err(ws_err)?;

        match msg_type[0] {
            0 => self.read_framebuffer_update(fb).await,
            1 => {
                let mut hdr = [0u8; 5];
                self.ws.read_exact(&mut hdr).await.map_err(ws_err)?;
                let num_colors = usize::from(u16::from_be_bytes([hdr[3], hdr[4]]));
                skip_reader_bytes(&mut self.ws, num_colors * 6).await?;
                Ok(RfbEvent::Other)
            }
            2 => Ok(RfbEvent::Other),
            3 => {
                let mut hdr = [0u8; 7];
                self.ws.read_exact(&mut hdr).await.map_err(ws_err)?;
                let len = u32::from_be_bytes([hdr[3], hdr[4], hdr[5], hdr[6]]) as usize;
                if len > MAX_SERVER_TEXT_BYTES {
                    return Err(ProxmoxError::VncFailed {
                        detail: "VNC ServerCutText exceeded size limit".to_owned(),
                    });
                }
                skip_reader_bytes(&mut self.ws, len).await?;
                Ok(RfbEvent::Other)
            }
            other => Err(ProxmoxError::VncFailed {
                detail: format!("unsupported RFB server message type {other}"),
            }),
        }
    }

    async fn read_framebuffer_update(
        &mut self,
        fb: &mut Framebuffer,
    ) -> Result<RfbEvent, ProxmoxError> {
        let mut hdr = [0u8; 3];
        self.ws.read_exact(&mut hdr).await.map_err(ws_err)?;
        let num_rects = u16::from_be_bytes([hdr[1], hdr[2]]);
        if num_rects > MAX_RECTS_PER_UPDATE {
            return Err(ProxmoxError::VncFailed {
                detail: "RFB FramebufferUpdate exceeded rectangle limit".to_owned(),
            });
        }

        let mut dirty: Option<Rect> = None;
        let mut resized = false;

        for _ in 0..num_rects {
            let (rect_dirty, rect_resized) = self.read_one_rectangle(fb).await?;
            resized |= rect_resized;
            if let Some(area) = rect_dirty {
                dirty = Some(match dirty {
                    Some(existing) => existing.union(area),
                    None => area,
                });
            }
        }

        if let Some(dirty_rect) = dirty {
            fb.commit_update();
            Ok(RfbEvent::Updated {
                dirty: dirty_rect,
                resized,
            })
        } else {
            Ok(RfbEvent::Other)
        }
    }

    async fn read_one_rectangle(
        &mut self,
        fb: &mut Framebuffer,
    ) -> Result<(Option<Rect>, bool), ProxmoxError> {
        let mut rhdr = [0u8; 12];
        self.ws.read_exact(&mut rhdr).await.map_err(ws_err)?;
        let rect = Rect {
            x: u16::from_be_bytes([rhdr[0], rhdr[1]]),
            y: u16::from_be_bytes([rhdr[2], rhdr[3]]),
            width: u16::from_be_bytes([rhdr[4], rhdr[5]]),
            height: u16::from_be_bytes([rhdr[6], rhdr[7]]),
        };
        let encoding = i32::from_be_bytes([rhdr[8], rhdr[9], rhdr[10], rhdr[11]]);

        match encoding {
            ENCODING_DESKTOP_SIZE => {
                fb.resize(rect.width, rect.height)?;
                Ok((
                    Some(Rect {
                        x: 0,
                        y: 0,
                        width: fb.width,
                        height: fb.height,
                    }),
                    true,
                ))
            }
            ENCODING_RAW => {
                if rect.width == 0 || rect.height == 0 {
                    return Ok((None, false));
                }

                let byte_len = usize::from(rect.width) * usize::from(rect.height) * 4;
                if byte_len > MAX_ZLIB_INFLATED_BYTES {
                    return Err(ProxmoxError::VncFailed {
                        detail: "RFB Raw rectangle exceeded pixel buffer limit".to_owned(),
                    });
                }

                let mut pixels = vec![0u8; byte_len];
                self.ws.read_exact(&mut pixels).await.map_err(ws_err)?;
                fb.apply_raw(rect, &pixels)?;
                Ok((Some(rect), false))
            }
            ENCODING_COPY_RECT => {
                let mut src = [0u8; 4];
                self.ws.read_exact(&mut src).await.map_err(ws_err)?;
                let src_x = u16::from_be_bytes([src[0], src[1]]);
                let src_y = u16::from_be_bytes([src[2], src[3]]);
                if rect.width == 0 || rect.height == 0 {
                    return Ok((None, false));
                }

                fb.apply_copy_rect(rect, src_x, src_y)?;
                Ok((Some(rect), false))
            }
            ENCODING_ZLIB => {
                let mut len_buf = [0u8; 4];
                self.ws.read_exact(&mut len_buf).await.map_err(ws_err)?;
                let compressed_len = u32::from_be_bytes(len_buf) as usize;
                if compressed_len > MAX_ZLIB_COMPRESSED_BYTES {
                    return Err(ProxmoxError::VncFailed {
                        detail: "RFB Zlib rectangle exceeded compressed size limit".to_owned(),
                    });
                }

                let mut compressed = vec![0u8; compressed_len];
                if compressed_len > 0 {
                    self.ws.read_exact(&mut compressed).await.map_err(ws_err)?;
                }

                let expected = usize::from(rect.width) * usize::from(rect.height) * 4;
                if expected > MAX_ZLIB_INFLATED_BYTES {
                    return Err(ProxmoxError::VncFailed {
                        detail: "RFB Zlib rectangle exceeded inflated size limit".to_owned(),
                    });
                }

                let pixels = inflate_bounded(&mut self.zlib, &compressed, expected)?;
                if rect.width == 0 || rect.height == 0 {
                    return Ok((None, false));
                }

                fb.apply_raw(rect, &pixels)?;
                Ok((Some(rect), false))
            }
            other => Err(ProxmoxError::VncFailed {
                detail: format!("unsupported RFB encoding {other}"),
            }),
        }
    }
}

async fn skip_reader_bytes<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    ws: &mut WsByteReader<R, W>,
    mut remaining: usize,
) -> Result<(), ProxmoxError> {
    let mut scratch = [0u8; 4096];
    while remaining > 0 {
        let take = remaining.min(scratch.len());
        ws.read_exact(&mut scratch[..take]).await.map_err(ws_err)?;
        remaining -= take;
    }
    Ok(())
}

/// Inflates a persistent RFB Zlib stream chunk into exactly `expected` bytes
/// while enforcing strict upper bounds against zip/zlib bombs (T-126).
pub(crate) fn inflate_bounded(
    decompressor: &mut Decompress,
    compressed: &[u8],
    expected: usize,
) -> Result<Vec<u8>, ProxmoxError> {
    if expected > MAX_ZLIB_INFLATED_BYTES {
        return Err(ProxmoxError::VncFailed {
            detail: "RFB Zlib inflated target exceeds safety bound".to_owned(),
        });
    }

    let mut out = vec![0u8; expected];
    let mut in_pos = 0usize;
    let mut out_pos = 0usize;

    while out_pos < expected {
        let before_in = decompressor.total_in();
        let before_out = decompressor.total_out();
        let status = decompressor
            .decompress(
                &compressed[in_pos..],
                &mut out[out_pos..],
                FlushDecompress::Sync,
            )
            .map_err(|error| ProxmoxError::VncFailed {
                detail: format!("RFB Zlib decompression failed: {error}"),
            })?;
        let consumed = usize::try_from(decompressor.total_in() - before_in).unwrap_or(0);
        let produced = usize::try_from(decompressor.total_out() - before_out).unwrap_or(0);
        in_pos += consumed;
        out_pos += produced;

        if out_pos == expected {
            break;
        }

        if consumed == 0 && produced == 0 {
            return Err(ProxmoxError::VncFailed {
                detail: "RFB Zlib stream truncated or stalled before rectangle completed"
                    .to_owned(),
            });
        }

        if matches!(status, Status::StreamEnd) {
            break;
        }
    }

    // Detect extra decompressed bytes (including buffered deflate output after
    // all compressed input bytes were consumed — catches zlib bombs, T-126).
    let mut extra = [0u8; 16];
    let before_out = decompressor.total_out();
    drop(decompressor.decompress(&compressed[in_pos..], &mut extra, FlushDecompress::Sync));
    if decompressor.total_out() > before_out {
        return Err(ProxmoxError::VncFailed {
            detail: "RFB Zlib payload decompressed past rectangle bounds".to_owned(),
        });
    }

    if out_pos != expected {
        return Err(ProxmoxError::VncFailed {
            detail: "RFB Zlib payload produced fewer bytes than rectangle requires".to_owned(),
        });
    }

    Ok(out)
}

fn validate_dimensions(width: u16, height: u16) -> Result<(), ProxmoxError> {
    let valid = width > 0 && height > 0 && width <= MAX_FB_WIDTH && height <= MAX_FB_HEIGHT;
    if !valid {
        return Err(ProxmoxError::VncFailed {
            detail: format!(
                "RFB framebuffer dimensions {width}x{height} are outside allowed bounds"
            ),
        });
    }
    Ok(())
}

async fn read_rfb_string<S: AsyncRead + AsyncWrite + Unpin>(
    ws: &mut WsByteStream<S>,
) -> Result<String, ProxmoxError> {
    let mut len_buf = [0u8; 4];
    ws.read_exact(&mut len_buf).await.map_err(ws_err)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_SERVER_TEXT_BYTES {
        return Err(ProxmoxError::VncFailed {
            detail: "RFB string length exceeded limit".to_owned(),
        });
    }

    let mut bytes = vec![0u8; len];
    if len > 0 {
        ws.read_exact(&mut bytes).await.map_err(ws_err)?;
    }

    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "used directly as a `Result::map_err` conversion function"
)]
fn ws_err(error: WsError) -> ProxmoxError {
    ProxmoxError::VncFailed {
        detail: error.to_string(),
    }
}

fn encode_rgba_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, ProxmoxError> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut out), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder
            .write_header()
            .map_err(|error| ProxmoxError::VncFailed {
                detail: format!("PNG header encoding failed: {error}"),
            })?;
        writer
            .write_image_data(rgba)
            .map_err(|error| ProxmoxError::VncFailed {
                detail: format!("PNG data encoding failed: {error}"),
            })?;
    }
    Ok(out)
}

/// Simple nearest-box downscaler for oversized VNC framebuffers when encoding
/// ADR-0044 tool screenshots.
fn downscale_rgba(src: &[u8], src_w: usize, src_h: usize, dst_w: usize, dst_h: usize) -> Vec<u8> {
    let mut dst = vec![0u8; dst_w * dst_h * 4];
    for dy in 0..dst_h {
        let sy = dy * src_h / dst_h;
        for dx in 0..dst_w {
            let sx = dx * src_w / dst_w;
            let s_idx = (sy * src_w + sx) * 4;
            let d_idx = (dy * dst_w + dx) * 4;
            dst[d_idx..d_idx + 4].copy_from_slice(&src[s_idx..s_idx + 4]);
        }
    }
    dst
}

// ---------------------------------------------------------------------------
// RFC 6143 §7.2.2 VNC Authentication: 56-bit DES-ECB with bit-reversed key
// ---------------------------------------------------------------------------

/// Encrypts a 16-byte RFB challenge using the first 8 bytes of `password`
/// (zero-padded, each byte bit-reversed per RFC 6143 / VNC spec).
pub(crate) fn vnc_des_encrypt_challenge(password: &[u8], challenge: &[u8; 16]) -> [u8; 16] {
    let mut key = [0u8; 8];
    for (index, slot) in key.iter_mut().enumerate() {
        if let Some(&byte) = password.get(index) {
            *slot = byte.reverse_bits();
        }
    }

    let subkeys = des_key_schedule(u64::from_be_bytes(key));
    let mut block1 = [0u8; 8];
    let mut block2 = [0u8; 8];
    block1.copy_from_slice(&challenge[..8]);
    block2.copy_from_slice(&challenge[8..]);

    let enc1 = des_encrypt_block(u64::from_be_bytes(block1), &subkeys).to_be_bytes();
    let enc2 = des_encrypt_block(u64::from_be_bytes(block2), &subkeys).to_be_bytes();
    let mut response = [0u8; 16];
    response[..8].copy_from_slice(&enc1);
    response[8..].copy_from_slice(&enc2);
    response
}

const IP: [u8; 64] = [
    58, 50, 42, 34, 26, 18, 10, 2, 60, 52, 44, 36, 28, 20, 12, 4, 62, 54, 46, 38, 30, 22, 14, 6,
    64, 56, 48, 40, 32, 24, 16, 8, 57, 49, 41, 33, 25, 17, 9, 1, 59, 51, 43, 35, 27, 19, 11, 3, 61,
    53, 45, 37, 29, 21, 13, 5, 63, 55, 47, 39, 31, 23, 15, 7,
];

const FP: [u8; 64] = [
    40, 8, 48, 16, 56, 24, 64, 32, 39, 7, 47, 15, 55, 23, 63, 31, 38, 6, 46, 14, 54, 22, 62, 30,
    37, 5, 45, 13, 53, 21, 61, 29, 36, 4, 44, 12, 52, 20, 60, 28, 35, 3, 43, 11, 51, 19, 59, 27,
    34, 2, 42, 10, 50, 18, 58, 26, 33, 1, 41, 9, 49, 17, 57, 25,
];

const E_TABLE: [u8; 48] = [
    32, 1, 2, 3, 4, 5, 4, 5, 6, 7, 8, 9, 8, 9, 10, 11, 12, 13, 12, 13, 14, 15, 16, 17, 16, 17, 18,
    19, 20, 21, 20, 21, 22, 23, 24, 25, 24, 25, 26, 27, 28, 29, 28, 29, 30, 31, 32, 1,
];

const P_TABLE: [u8; 32] = [
    16, 7, 20, 21, 29, 12, 28, 17, 1, 15, 23, 26, 5, 18, 31, 10, 2, 8, 24, 14, 32, 27, 3, 9, 19,
    13, 30, 6, 22, 11, 4, 25,
];

const PC1: [u8; 56] = [
    57, 49, 41, 33, 25, 17, 9, 1, 58, 50, 42, 34, 26, 18, 10, 2, 59, 51, 43, 35, 27, 19, 11, 3, 60,
    52, 44, 36, 63, 55, 47, 39, 31, 23, 15, 7, 62, 54, 46, 38, 30, 22, 14, 6, 61, 53, 45, 37, 29,
    21, 13, 5, 28, 20, 12, 4,
];

const PC2: [u8; 48] = [
    14, 17, 11, 24, 1, 5, 3, 28, 15, 6, 21, 10, 23, 19, 12, 4, 26, 8, 16, 7, 27, 20, 13, 2, 41, 52,
    31, 37, 47, 55, 30, 40, 51, 45, 33, 48, 44, 49, 39, 56, 34, 53, 46, 42, 50, 36, 29, 32,
];

const SHIFTS: [u32; 16] = [1, 1, 2, 2, 2, 2, 2, 2, 1, 2, 2, 2, 2, 2, 2, 1];

const SBOX: [[u8; 64]; 8] = [
    [
        14, 4, 13, 1, 2, 15, 11, 8, 3, 10, 6, 12, 5, 9, 0, 7, 0, 15, 7, 4, 14, 2, 13, 1, 10, 6, 12,
        11, 9, 5, 3, 8, 4, 1, 14, 8, 13, 6, 2, 11, 15, 12, 9, 7, 3, 10, 5, 0, 15, 12, 8, 2, 4, 9,
        1, 7, 5, 11, 3, 14, 10, 0, 6, 13,
    ],
    [
        15, 1, 8, 14, 6, 11, 3, 4, 9, 7, 2, 13, 12, 0, 5, 10, 3, 13, 4, 7, 15, 2, 8, 14, 12, 0, 1,
        10, 6, 9, 11, 5, 0, 14, 7, 11, 10, 4, 13, 1, 5, 8, 12, 6, 9, 3, 2, 15, 13, 8, 10, 1, 3, 15,
        4, 2, 11, 6, 7, 12, 0, 5, 14, 9,
    ],
    [
        10, 0, 9, 14, 6, 3, 15, 5, 1, 13, 12, 7, 11, 4, 2, 8, 13, 7, 0, 9, 3, 4, 6, 10, 2, 8, 5,
        14, 12, 11, 15, 1, 13, 6, 4, 9, 8, 15, 3, 0, 11, 1, 2, 12, 5, 10, 14, 7, 1, 10, 13, 0, 6,
        9, 8, 7, 4, 15, 14, 3, 11, 5, 2, 12,
    ],
    [
        7, 13, 14, 3, 0, 6, 9, 10, 1, 2, 8, 5, 11, 12, 4, 15, 13, 8, 11, 5, 6, 15, 0, 3, 4, 7, 2,
        12, 1, 10, 14, 9, 10, 6, 9, 0, 12, 11, 7, 13, 15, 1, 3, 14, 5, 2, 8, 4, 3, 15, 0, 6, 10, 1,
        13, 8, 9, 4, 5, 11, 12, 7, 2, 14,
    ],
    [
        2, 12, 4, 1, 7, 10, 11, 6, 8, 5, 3, 15, 13, 0, 14, 9, 14, 11, 2, 12, 4, 7, 13, 1, 5, 0, 15,
        10, 3, 9, 8, 6, 4, 2, 1, 11, 10, 13, 7, 8, 15, 9, 12, 5, 6, 3, 0, 14, 11, 8, 12, 7, 1, 14,
        2, 13, 6, 15, 0, 9, 10, 4, 5, 3,
    ],
    [
        12, 1, 10, 15, 9, 2, 6, 8, 0, 13, 3, 4, 14, 7, 5, 11, 10, 15, 4, 2, 7, 12, 9, 5, 6, 1, 13,
        14, 0, 11, 3, 8, 9, 14, 15, 5, 2, 8, 12, 3, 7, 0, 4, 10, 1, 13, 11, 6, 4, 3, 2, 12, 9, 5,
        15, 10, 11, 14, 1, 7, 6, 0, 8, 13,
    ],
    [
        4, 11, 2, 14, 15, 0, 8, 13, 3, 12, 9, 7, 5, 10, 6, 1, 13, 0, 11, 7, 4, 9, 1, 10, 14, 3, 5,
        12, 2, 15, 8, 6, 1, 4, 11, 13, 12, 3, 7, 14, 10, 15, 6, 8, 0, 5, 9, 2, 6, 11, 13, 8, 1, 4,
        10, 7, 9, 5, 0, 15, 14, 2, 3, 12,
    ],
    [
        13, 2, 8, 4, 6, 15, 11, 1, 10, 9, 3, 14, 5, 0, 12, 7, 1, 15, 13, 8, 10, 3, 7, 4, 12, 5, 6,
        11, 0, 14, 9, 2, 7, 11, 4, 1, 9, 12, 14, 2, 0, 6, 10, 13, 15, 3, 5, 8, 2, 1, 14, 7, 4, 10,
        8, 13, 15, 12, 9, 0, 3, 5, 6, 11,
    ],
];

fn permute(input: u64, table: &[u8], input_bits: u8) -> u64 {
    let mut out = 0u64;
    for &pos in table {
        let bit = (input >> (input_bits - pos)) & 1;
        out = (out << 1) | bit;
    }
    out
}

fn des_key_schedule(key: u64) -> [u64; 16] {
    let permuted = permute(key, &PC1, 64);
    let mut c = (permuted >> 28) & 0x0fff_ffff;
    let mut d = permuted & 0x0fff_ffff;
    let mut subkeys = [0u64; 16];

    for (round, &shift) in SHIFTS.iter().enumerate() {
        c = ((c << shift) | (c >> (28 - shift))) & 0x0fff_ffff;
        d = ((d << shift) | (d >> (28 - shift))) & 0x0fff_ffff;
        subkeys[round] = permute((c << 28) | d, &PC2, 56);
    }

    subkeys
}

fn des_feistel(r: u32, subkey: u64) -> u32 {
    let expanded = permute(u64::from(r), &E_TABLE, 32) ^ subkey;
    let mut substituted = 0u32;
    for (box_idx, sbox) in SBOX.iter().enumerate() {
        let shift = (7 - box_idx) * 6;
        let chunk = ((expanded >> shift) & 0x3f) as usize;
        let row = ((chunk & 0x20) >> 4) | (chunk & 0x01);
        let col = (chunk >> 1) & 0x0f;
        let val = u32::from(sbox[row * 16 + col]);
        substituted = (substituted << 4) | val;
    }

    #[expect(clippy::cast_possible_truncation, reason = "P_TABLE outputs 32 bits")]
    {
        permute(u64::from(substituted), &P_TABLE, 32) as u32
    }
}

fn des_encrypt_block(block: u64, subkeys: &[u64; 16]) -> u64 {
    let ip = permute(block, &IP, 64);
    #[expect(clippy::cast_possible_truncation, reason = "high/low 32-bit halves")]
    let (mut l, mut r) = ((ip >> 32) as u32, ip as u32);

    for &subkey in subkeys {
        let next_r = l ^ des_feistel(r, subkey);
        l = r;
        r = next_r;
    }

    let preoutput = (u64::from(r) << 32) | u64::from(l);
    permute(preoutput, &FP, 64)
}

#[cfg(test)]
mod tests {
    use flate2::{Compress, Compression, Decompress, FlushCompress};

    use super::{
        Framebuffer, Rect, des_encrypt_block, des_key_schedule, inflate_bounded,
        vnc_des_encrypt_challenge,
    };

    #[test]
    fn des_matches_nist_fips_46_3_vector() {
        // Standard FIPS 46-3 / NBS SP 500-20 test vector.
        let key = 0x1334_5779_9bbc_dff1u64;
        let plain = 0x0123_4567_89ab_cdefu64;
        let subkeys = des_key_schedule(key);
        assert_eq!(des_encrypt_block(plain, &subkeys), 0x85e8_1354_0f0a_b405u64);
    }

    #[test]
    fn vnc_des_auth_reverses_key_bits_and_encrypts_both_blocks() {
        // Verify deterministic RFC 6143 SecurityType 2 encryption:
        // password "password" -> bit-reversed bytes match known DES output.
        let challenge = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10,
        ];
        let resp = vnc_des_encrypt_challenge(b"secret12", &challenge);
        let resp_same = vnc_des_encrypt_challenge(b"secret12extraignored", &challenge);
        assert_eq!(resp, resp_same);
        assert_ne!(resp, challenge);
    }

    #[test]
    fn framebuffer_raw_copy_rect_zlib_and_png_work_and_reject_oversized() {
        drop(Framebuffer::new(0, 10).unwrap_err());
        drop(Framebuffer::new(5000, 100).unwrap_err());

        let mut fb = Framebuffer::new(8, 8).unwrap();
        let rect = Rect {
            x: 1,
            y: 1,
            width: 2,
            height: 2,
        };
        let red = [255, 0, 0, 0].repeat(4);
        fb.apply_raw(rect, &red).unwrap();
        fb.apply_copy_rect(
            Rect {
                x: 4,
                y: 4,
                width: 2,
                height: 2,
            },
            1,
            1,
        )
        .unwrap();
        fb.commit_update();

        // Alpha is forced opaque (255).
        let idx = (4 * 8 + 4) * 4;
        assert_eq!(&fb.rgba[idx..idx + 4], &[255, 0, 0, 255]);

        let (png_bytes, w, h) = fb.encode_tool_screenshot().unwrap();
        assert_eq!((w, h), (8, 8));
        assert!(png_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));

        // Bounded zlib inflation rejects trailing bomb output.
        let mut comp = Compress::new(Compression::default(), true);
        let input = vec![0x42u8; 1024];
        let mut compressed = vec![0u8; 2048];
        comp.compress(&input, &mut compressed, FlushCompress::Sync)
            .unwrap();
        compressed.truncate(usize::try_from(comp.total_out()).unwrap());

        let mut decomp = Decompress::new(true);
        let ok = inflate_bounded(&mut decomp, &compressed, 1024).unwrap();
        assert_eq!(ok, input);

        let mut decomp2 = Decompress::new(true);
        drop(inflate_bounded(&mut decomp2, &compressed, 16).unwrap_err());
    }
}
