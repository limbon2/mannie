//! Live VNC session manager (ADR-0080, decisions 5 & 7):
//! - Sessions are keyed by `(ConversationId, server_id, vmid)`.
//! - Concurrent session cap (`MAX_VNC_SESSIONS = 8`) and idle reaper.
//! - Explicit control ownership (`Agent` vs `Owner`):
//!   - Under `Owner` takeover, ALL agent VNC operations (including screenshots)
//!     are refused in Rust (`owner_in_control`) until the owner resumes.
//!   - Under `Agent` control, ALL owner input IPC calls are refused in Rust
//!     (`agent_in_control`).
//! - Live UI streaming (`follow_vnc_session`) encodes dirty-rect PNG patches at
//!   most once every 50 ms (~20 fps) and performs ZERO PNG encoding when no
//!   live panel subscriber is connected.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use base64::Engine as _;
use futures_util::{StreamExt as _, stream};
use mannie_plugin_sdk::contracts::{ConversationId, Secret};
use mannie_plugin_sdk::ipc::PluginStream;
use tokio::io::WriteHalf;
use tokio::net::TcpStream;
use tokio::sync::{Notify, oneshot};
use tokio_rustls::client::TlsStream;

use crate::api::{ProxmoxApiClient, build_vnc_tls_config, encode_vnc_ticket};
use crate::keysyms::{KEYSYM_KP_ENTER, KEYSYM_RETURN, KeyStroke, ctrl_alt_delete};
use crate::model::{
    ProxmoxError, ServerRecord, VncControlOwner, VncSessionState, VncSessionSummary,
    VncSessionUpdate,
};
use crate::rfb::{Framebuffer, Rect, RfbClient, RfbEvent, RfbWriter};
use crate::tls::connect_tls;
use crate::ws::WsByteStream;

/// Maximum concurrent live VNC sessions across all conversations.
pub(crate) const MAX_VNC_SESSIONS: usize = 8;

/// Default idle timeout after which an unused VNC session closes automatically.
pub(crate) const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_mins(10);

/// Handshake timeout when opening a VNC WebSocket + RFB 3.8 session.
const VNC_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// Wait for the first framebuffer update after `ServerInit` before returning a screenshot.
const INITIAL_FRAME_WAIT: Duration = Duration::from_millis(1500);

/// Maximum `wait_for_change_ms` accepted on VNC tools.
pub(crate) const MAX_WAIT_FOR_CHANGE_MS: u64 = 10_000;

/// Maximum unconditional `delay_ms` accepted on `proxmox_vnc_screenshot`.
pub(crate) const MAX_SCREENSHOT_DELAY_MS: u64 = 60_000;

/// Delay between press and release of synthetic agent keystrokes / clicks.
const INPUT_PULSE_DELAY: Duration = Duration::from_millis(20);

/// Settle delay between modifier transitions (`Shift_L`, `Ctrl`, `Alt`) and the
/// primary key so QEMU's emulated PS/2 controller preserves ordering.
const MODIFIER_SETTLE_DELAY: Duration = Duration::from_millis(8);

/// Gap between consecutive synthetic keystrokes so QEMU's 16-byte PS/2 i8042
/// FIFO never overruns during long typed commands.
const INTER_KEY_DELAY: Duration = Duration::from_millis(12);

/// Default wait after `Enter` or pointer actions when `wait_for_change_ms` is
/// omitted and a screenshot is requested, preventing stale pre-action frames.
const DEFAULT_POST_ACTION_WAIT: Duration = Duration::from_millis(500);

/// Post-typing wait for the guest console to paint the final typed characters
/// before capturing a screenshot when no trailing `Enter` was sent.
const POST_TYPING_ECHO_WAIT: Duration = Duration::from_millis(250);

/// Brief quiet period after a framebuffer change so multi-character console
/// bursts finish painting before taking a screenshot.
const POST_INPUT_QUIET_DELAY: Duration = Duration::from_millis(80);

/// Cap on the post-change quiet settle window.
const POST_INPUT_SETTLE_CAP: Duration = Duration::from_millis(250);

/// Explicit `wait_for_change_ms` threshold above which the post-change quiet
/// settle window widens so multi-stage redraws (such as Tab highlight followed
/// by Enter page switch) finish before capture.
const LONG_WAIT_THRESHOLD_MS: u64 = 2_000;

/// Quiet period used when the caller passed a large explicit `wait_for_change_ms`.
const LONG_INPUT_QUIET_DELAY: Duration = Duration::from_millis(150);

/// Cap on the settle window when the caller passed a large explicit `wait_for_change_ms`.
const LONG_INPUT_SETTLE_CAP: Duration = Duration::from_millis(500);

/// Throttle interval for live VNC side-panel PNG patch streaming (~20 fps).
const LIVE_STREAM_INTERVAL: Duration = Duration::from_millis(50);

/// Unique key of a live VNC session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SessionKey {
    pub(crate) conversation_id: ConversationId,
    pub(crate) server_id: String,
    pub(crate) vmid: u32,
}

/// Captured screenshot returned to agent VNC tools.
#[derive(Debug, Clone)]
pub(crate) struct VncScreenshot {
    pub(crate) png_bytes: Vec<u8>,
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) changed: bool,
    pub(crate) seq: u64,
}

#[derive(Debug)]
struct SessionShared {
    server_name: String,
    node: String,
    vm_name: String,
    control: VncControlOwner,
    state: VncSessionState,
    closed_reason: Option<String>,
    framebuffer: Framebuffer,
    last_activity: Instant,
    /// Pending dirty rectangle accumulated since the last stream flush.
    /// Cleared whenever the stream subscriber emits a snapshot/patch.
    stream_dirty: Option<Rect>,
    /// True when the framebuffer resized and subscribers need a full Snapshot.
    stream_needs_snapshot: bool,
    /// Monotonic counter bumped whenever control or metadata changes.
    status_version: u64,
}

#[derive(Debug)]
struct LiveSession {
    key: SessionKey,
    shared: Arc<Mutex<SessionShared>>,
    writer: RfbWriter<WriteHalf<TlsStream<TcpStream>>>,
    frame_notify: Arc<Notify>,
    subscribers: Arc<AtomicUsize>,
    shutdown: Mutex<Option<oneshot::Sender<String>>>,
}

impl LiveSession {
    fn summary(&self) -> VncSessionSummary {
        let guard = lock(&self.shared);
        VncSessionSummary {
            conversation_id: self.key.conversation_id.clone(),
            server_id: self.key.server_id.clone(),
            server_name: guard.server_name.clone(),
            node: guard.node.clone(),
            vmid: self.key.vmid,
            vm_name: guard.vm_name.clone(),
            control: guard.control,
            state: guard.state,
            width: guard.framebuffer.width,
            height: guard.framebuffer.height,
        }
    }

    fn touch(&self) {
        lock(&self.shared).last_activity = Instant::now();
    }

    fn ensure_agent_control(&self) -> Result<(), ProxmoxError> {
        let guard = lock(&self.shared);
        if guard.state == VncSessionState::Closed {
            return Err(ProxmoxError::SessionNotFound);
        }

        if guard.control != VncControlOwner::Agent {
            return Err(ProxmoxError::OwnerInControl);
        }

        Ok(())
    }

    fn ensure_owner_control(&self) -> Result<(), ProxmoxError> {
        let guard = lock(&self.shared);
        if guard.state == VncSessionState::Closed {
            return Err(ProxmoxError::SessionNotFound);
        }

        if guard.control != VncControlOwner::Owner {
            return Err(ProxmoxError::AgentInControl);
        }

        Ok(())
    }

    fn stop(&self, reason: &str) {
        {
            let mut guard = lock(&self.shared);
            guard.state = VncSessionState::Closed;
            if guard.closed_reason.is_none() {
                guard.closed_reason = Some(reason.to_owned());
            }
            guard.status_version = guard.status_version.saturating_add(1);
        }
        let tx = lock(&self.shutdown).take();
        if let Some(tx) = tx {
            drop(tx.send(reason.to_owned()));
        }
        self.frame_notify.notify_waiters();
    }
}

/// Outcome of trying to insert a freshly handshaken session into the map.
enum InsertOutcome {
    Existing(Arc<LiveSession>),
    Full,
    Inserted,
}

/// Manages all live VNC sessions across conversations.
#[derive(Debug, Clone)]
pub(crate) struct VncManager {
    sessions: Arc<Mutex<HashMap<SessionKey, Arc<LiveSession>>>>,
    idle_timeout: Duration,
}

impl VncManager {
    /// Creates a new VNC session manager with the given idle timeout.
    pub(crate) fn new(idle_timeout: Duration) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            idle_timeout,
        }
    }

    /// Number of currently open VNC sessions (reported by `Instance::work`).
    pub(crate) fn active_count(&self) -> u32 {
        let count = lock(&self.sessions)
            .values()
            .filter(|s| lock(&s.shared).state != VncSessionState::Closed)
            .count();
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    /// Lists open VNC sessions belonging to `conversation_id`.
    pub(crate) fn list_for_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Vec<VncSessionSummary> {
        let mut out: Vec<VncSessionSummary> = lock(&self.sessions)
            .values()
            .filter(|s| s.key.conversation_id == *conversation_id)
            .filter(|s| lock(&s.shared).state != VncSessionState::Closed)
            .map(|s| s.summary())
            .collect();
        out.sort_by(|a, b| a.server_name.cmp(&b.server_name).then(a.vmid.cmp(&b.vmid)));
        out
    }

    /// Ensures a live VNC session exists for `(conversation_id, server, vmid)`.
    /// When creating a new session, sets initial control to `initial_control`.
    /// When an existing session is already open and `for_agent` is true, rejects
    /// immediately with `OwnerInControl` if the owner currently controls it.
    #[expect(
        clippy::too_many_arguments,
        reason = "session creation needs target metadata, API client, password and initial ownership"
    )]
    pub(crate) async fn ensure_session(
        &self,
        conversation_id: ConversationId,
        server: &ServerRecord,
        node: &str,
        vmid: u32,
        vm_name: &str,
        api: &ProxmoxApiClient,
        password: &Secret,
        initial_control: VncControlOwner,
        for_agent: bool,
    ) -> Result<VncSessionSummary, ProxmoxError> {
        let key = SessionKey {
            conversation_id,
            server_id: server.id.clone(),
            vmid,
        };

        if let Some(existing) = self.get_live(&key) {
            if for_agent {
                existing.ensure_agent_control()?;
            }
            existing.touch();
            return Ok(existing.summary());
        }

        // Enforce concurrent session cap before opening a network connection.
        {
            let map = lock(&self.sessions);
            let live_count = map
                .values()
                .filter(|s| lock(&s.shared).state != VncSessionState::Closed)
                .count();
            if live_count >= MAX_VNC_SESSIONS {
                return Err(ProxmoxError::TooManySessions);
            }
        }

        let (rfb_reader, rfb_writer, framebuffer) =
            open_rfb_connection(server, node, vmid, api, password).await?;

        let shared = Arc::new(Mutex::new(SessionShared {
            server_name: server.name.clone(),
            node: node.to_owned(),
            vm_name: vm_name.to_owned(),
            control: initial_control,
            state: VncSessionState::Connected,
            closed_reason: None,
            framebuffer,
            last_activity: Instant::now(),
            stream_dirty: None,
            stream_needs_snapshot: true,
            status_version: 1,
        }));
        let frame_notify = Arc::new(Notify::new());
        let subscribers = Arc::new(AtomicUsize::new(0));
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<String>();

        let session = Arc::new(LiveSession {
            key: key.clone(),
            shared: Arc::clone(&shared),
            writer: rfb_writer.clone(),
            frame_notify: Arc::clone(&frame_notify),
            subscribers: Arc::clone(&subscribers),
            shutdown: Mutex::new(Some(shutdown_tx)),
        });

        // Insert into map (re-checking races and session cap) without holding
        // the std Mutex across any `.await`.
        let outcome = {
            let mut map = lock(&self.sessions);
            map.retain(|_, s| lock(&s.shared).state != VncSessionState::Closed);
            if let Some(existing) = map.get(&key) {
                InsertOutcome::Existing(Arc::clone(existing))
            } else if map.len() >= MAX_VNC_SESSIONS {
                InsertOutcome::Full
            } else {
                map.insert(key.clone(), Arc::clone(&session));
                InsertOutcome::Inserted
            }
        };

        match outcome {
            InsertOutcome::Existing(existing) => {
                rfb_writer.close().await;
                if for_agent {
                    existing.ensure_agent_control()?;
                }
                existing.touch();
                return Ok(existing.summary());
            }
            InsertOutcome::Full => {
                rfb_writer.close().await;
                return Err(ProxmoxError::TooManySessions);
            }
            InsertOutcome::Inserted => {}
        }

        spawn_rfb_reader_loop(
            Arc::clone(&self.sessions),
            key,
            shared,
            frame_notify,
            subscribers,
            rfb_reader,
            rfb_writer,
            shutdown_rx,
        );

        // Wait briefly for the first framebuffer paint so the first screenshot isn't blank.
        let _ = self.wait_for_seq(&session, 1, INITIAL_FRAME_WAIT).await;
        Ok(session.summary())
    }

    /// Captures a PNG screenshot for an agent tool call, optionally sleeping an
    /// unconditional `delay_ms` (for slow boots/installs with spinners) and/or
    /// waiting up to `wait_for_change_ms` for the framebuffer hash to change.
    /// Refuses with `OwnerInControl` if the owner holds console takeover.
    pub(crate) async fn agent_screenshot(
        &self,
        key: &SessionKey,
        delay_ms: Option<u64>,
        wait_for_change_ms: Option<u64>,
    ) -> Result<VncScreenshot, ProxmoxError> {
        let session = self.get_live(key).ok_or(ProxmoxError::SessionNotFound)?;
        session.ensure_agent_control()?;
        session.touch();

        let pre_delay_hash = lock(&session.shared).framebuffer.hash;
        if let Some(ms) = delay_ms.filter(|&ms| ms > 0) {
            let capped = ms.min(MAX_SCREENSHOT_DELAY_MS);
            self.sleep_while_agent_controls(&session, Duration::from_millis(capped))
                .await?;
            session.touch();
        }

        let (initial_hash, initial_seq, w, h) = {
            let guard = lock(&session.shared);
            (
                guard.framebuffer.hash,
                guard.framebuffer.seq,
                guard.framebuffer.width,
                guard.framebuffer.height,
            )
        };

        // Request an incremental refresh before waiting.
        drop(
            session
                .writer
                .send_framebuffer_update_request(true, 0, 0, w, h)
                .await,
        );

        let waited_change = if let Some(wait_ms) = wait_for_change_ms.filter(|&ms| ms > 0) {
            let changed = self
                .wait_for_hash_change(
                    &session,
                    initial_hash,
                    Duration::from_millis(wait_ms.min(MAX_WAIT_FOR_CHANGE_MS)),
                )
                .await;
            if changed {
                let (quiet, cap) = settle_windows(wait_for_change_ms);
                self.settle_framebuffer(&session, quiet, cap).await;
            }
            changed
        } else if initial_seq == 0 {
            let _ = self
                .wait_for_seq(&session, 1, Duration::from_millis(500))
                .await;
            lock(&session.shared).framebuffer.hash != initial_hash
        } else {
            false
        };

        // Re-check owner takeover after waiting so an agent never reads a
        // frame produced while the owner took over (ADR-0080 / D74).
        session.ensure_agent_control()?;
        let (png_bytes, out_w, out_h, seq, final_hash) = {
            let guard = lock(&session.shared);
            let (bytes, out_w, out_h) = guard.framebuffer.encode_tool_screenshot()?;
            (
                bytes,
                out_w,
                out_h,
                guard.framebuffer.seq,
                guard.framebuffer.hash,
            )
        };
        let changed = waited_change || final_hash != pre_delay_hash;

        Ok(VncScreenshot {
            png_bytes,
            width: out_w,
            height: out_h,
            changed,
            seq,
        })
    }

    /// Sends an agent pointer gesture (`move`, `click`, `double_click`,
    /// `right_click`, `middle_click`, `down`, `up`, `scroll`) and optionally
    /// captures an updated screenshot.
    #[expect(
        clippy::too_many_arguments,
        reason = "pointer gesture parameters plus optional post-action screenshot"
    )]
    pub(crate) async fn agent_pointer(
        &self,
        key: &SessionKey,
        action: &str,
        x: u16,
        y: u16,
        button: Option<&str>,
        scroll_delta: Option<i32>,
        delay_ms: Option<u64>,
        wait_for_change_ms: Option<u64>,
        take_screenshot: bool,
    ) -> Result<Option<VncScreenshot>, ProxmoxError> {
        let session = self.get_live(key).ok_or(ProxmoxError::SessionNotFound)?;
        session.ensure_agent_control()?;
        session.touch();

        let (fb_w, fb_h, before_hash) = {
            let guard = lock(&session.shared);
            (
                guard.framebuffer.width,
                guard.framebuffer.height,
                guard.framebuffer.hash,
            )
        };
        let clamped_x = x.min(fb_w.saturating_sub(1));
        let clamped_y = y.min(fb_h.saturating_sub(1));

        match action {
            "move" | "up" => {
                session
                    .writer
                    .send_pointer_event(clamped_x, clamped_y, 0)
                    .await?;
            }
            "click" => {
                let mask = button_mask(button.unwrap_or("left"))?;
                send_click(&session.writer, clamped_x, clamped_y, mask).await?;
            }
            "double_click" => {
                let mask = button_mask(button.unwrap_or("left"))?;
                send_click(&session.writer, clamped_x, clamped_y, mask).await?;
                tokio::time::sleep(Duration::from_millis(50)).await;
                session.ensure_agent_control()?;
                send_click(&session.writer, clamped_x, clamped_y, mask).await?;
            }
            "right_click" => {
                send_click(&session.writer, clamped_x, clamped_y, 0x04).await?;
            }
            "middle_click" => {
                send_click(&session.writer, clamped_x, clamped_y, 0x02).await?;
            }
            "down" => {
                let mask = button_mask(button.unwrap_or("left"))?;
                session
                    .writer
                    .send_pointer_event(clamped_x, clamped_y, mask)
                    .await?;
            }
            "scroll" => {
                let delta = scroll_delta.unwrap_or(-3);
                let mask = if delta > 0 { 0x08 } else { 0x10 };
                let steps = delta.unsigned_abs().clamp(1, 20);
                for _ in 0..steps {
                    session.ensure_agent_control()?;
                    send_click(&session.writer, clamped_x, clamped_y, mask).await?;
                }
            }
            other => {
                return Err(ProxmoxError::InvalidRequest {
                    detail: format!("unsupported pointer action {other:?}"),
                });
            }
        }

        self.finish_agent_input(
            &session,
            before_hash,
            delay_ms,
            wait_for_change_ms,
            take_screenshot,
        )
        .await
    }

    /// Sends a sequence of agent keystrokes and optionally captures an updated screenshot.
    pub(crate) async fn agent_keyboard(
        &self,
        key: &SessionKey,
        strokes: &[KeyStroke],
        delay_ms: Option<u64>,
        wait_for_change_ms: Option<u64>,
        take_screenshot: bool,
    ) -> Result<Option<VncScreenshot>, ProxmoxError> {
        let session = self.get_live(key).ok_or(ProxmoxError::SessionNotFound)?;
        session.ensure_agent_control()?;
        session.touch();

        let before_hash = lock(&session.shared).framebuffer.hash;
        let ends_with_submit = strokes
            .last()
            .is_some_and(|stroke| matches!(stroke.key, KEYSYM_RETURN | KEYSYM_KP_ENTER));
        let mut wait_base_hash = before_hash;

        for (index, stroke) in strokes.iter().enumerate() {
            session.ensure_agent_control()?;
            let is_last = index + 1 == strokes.len();
            if is_last && ends_with_submit {
                // Measure command-output changes relative to the screen right
                // before Enter rather than before the first typed character.
                wait_base_hash = lock(&session.shared).framebuffer.hash;
            }
            send_keystroke(&session.writer, stroke).await?;
            if !is_last {
                tokio::time::sleep(INTER_KEY_DELAY).await;
            }
        }

        if !ends_with_submit {
            wait_base_hash = lock(&session.shared).framebuffer.hash;
        }

        self.finish_agent_keyboard(
            &session,
            before_hash,
            wait_base_hash,
            ends_with_submit,
            delay_ms,
            wait_for_change_ms,
            take_screenshot,
        )
        .await
    }

    /// Closes a session on behalf of an agent tool; refused if the owner holds takeover.
    pub(crate) fn agent_close(&self, key: &SessionKey) -> Result<(), ProxmoxError> {
        let session = self.get_live(key).ok_or(ProxmoxError::SessionNotFound)?;
        session.ensure_agent_control()?;
        session.stop("agent_closed");
        lock(&self.sessions).remove(key);
        Ok(())
    }

    /// Owner switches live console control between `owner` (takeover) and
    /// `agent` (resume).
    pub(crate) fn set_control(
        &self,
        key: &SessionKey,
        control: VncControlOwner,
    ) -> Result<VncSessionSummary, ProxmoxError> {
        let session = self.get_live(key).ok_or(ProxmoxError::SessionNotFound)?;
        {
            let mut guard = lock(&session.shared);
            guard.control = control;
            guard.last_activity = Instant::now();
            guard.status_version = guard.status_version.saturating_add(1);
        }
        session.frame_notify.notify_waiters();
        Ok(session.summary())
    }

    /// Sends an owner pointer event from the live VNC panel (requires `Owner` mode).
    pub(crate) async fn owner_pointer(
        &self,
        key: &SessionKey,
        x: u16,
        y: u16,
        button_mask: u8,
    ) -> Result<(), ProxmoxError> {
        let session = self.get_live(key).ok_or(ProxmoxError::SessionNotFound)?;
        session.ensure_owner_control()?;
        session.touch();
        let (fb_w, fb_h) = {
            let guard = lock(&session.shared);
            (guard.framebuffer.width, guard.framebuffer.height)
        };
        session
            .writer
            .send_pointer_event(
                x.min(fb_w.saturating_sub(1)),
                y.min(fb_h.saturating_sub(1)),
                button_mask,
            )
            .await
    }

    /// Sends an owner keyboard event from the live VNC panel (requires `Owner` mode).
    pub(crate) async fn owner_keyboard(
        &self,
        key: &SessionKey,
        keysym: u32,
        down: bool,
    ) -> Result<(), ProxmoxError> {
        let session = self.get_live(key).ok_or(ProxmoxError::SessionNotFound)?;
        session.ensure_owner_control()?;
        session.touch();
        session.writer.send_key_event(keysym, down).await
    }

    /// Sends `Ctrl+Alt+Delete` from the live VNC panel toolbar (requires `Owner` mode).
    pub(crate) async fn owner_ctrl_alt_del(&self, key: &SessionKey) -> Result<(), ProxmoxError> {
        let session = self.get_live(key).ok_or(ProxmoxError::SessionNotFound)?;
        session.ensure_owner_control()?;
        session.touch();
        send_keystroke(&session.writer, &ctrl_alt_delete()).await
    }

    /// Closes a VNC session on behalf of the owner.
    pub(crate) fn owner_close(&self, key: &SessionKey) {
        let removed = lock(&self.sessions).remove(key);
        if let Some(session) = removed {
            session.stop("owner_closed");
        }
    }

    /// Opens a live streaming channel for the VNC side panel. Increments the
    /// session subscriber count while alive (and decrements on drop) so zero
    /// PNG encoding happens when no panel is watching.
    pub(crate) fn follow_session(
        &self,
        key: &SessionKey,
    ) -> Result<PluginStream<VncSessionUpdate>, ProxmoxError> {
        let session = self.get_live(key).ok_or(ProxmoxError::SessionNotFound)?;
        let guard = SubscriberGuard::new(Arc::clone(&session.subscribers));
        let initial_state = StreamState {
            session,
            _guard: guard,
            sent_initial: false,
            last_seq: 0,
            last_status_version: 0,
            ended: false,
        };

        Ok(stream::unfold(initial_state, next_stream_update).boxed())
    }

    /// Closes all sessions belonging to any deleted conversation (ADR-0080, decision 6).
    pub(crate) fn close_conversations(&self, conversations: &[ConversationId]) {
        let mut to_stop = Vec::new();
        {
            let mut map = lock(&self.sessions);
            map.retain(|key, session| {
                if conversations.contains(&key.conversation_id) {
                    to_stop.push(Arc::clone(session));
                    false
                } else {
                    true
                }
            });
        }
        for session in to_stop {
            session.stop("conversation_deleted");
        }
    }

    /// Closes all sessions for a specific VM (e.g. when unchecked from allowlist).
    pub(crate) fn close_vm(&self, server_id: &str, vmid: u32, reason: &str) {
        let mut to_stop = Vec::new();
        {
            let mut map = lock(&self.sessions);
            map.retain(|key, session| {
                if key.server_id == server_id && key.vmid == vmid {
                    to_stop.push(Arc::clone(session));
                    false
                } else {
                    true
                }
            });
        }
        for session in to_stop {
            session.stop(reason);
        }
    }

    /// Closes all sessions targeting `server_id` (on server edit or removal).
    pub(crate) fn close_server(&self, server_id: &str, reason: &str) {
        let mut to_stop = Vec::new();
        {
            let mut map = lock(&self.sessions);
            map.retain(|key, session| {
                if key.server_id == server_id {
                    to_stop.push(Arc::clone(session));
                    false
                } else {
                    true
                }
            });
        }
        for session in to_stop {
            session.stop(reason);
        }
    }

    /// Closes sessions idle longer than `self.idle_timeout`.
    pub(crate) fn close_idle(&self) {
        let now = Instant::now();
        let mut to_stop = Vec::new();
        {
            let mut map = lock(&self.sessions);
            map.retain(|_, session| {
                let guard = lock(&session.shared);
                let has_watchers = session.subscribers.load(Ordering::Relaxed) > 0;
                let expired = !has_watchers
                    && now.saturating_duration_since(guard.last_activity) >= self.idle_timeout;
                if expired || guard.state == VncSessionState::Closed {
                    to_stop.push(Arc::clone(session));
                    false
                } else {
                    true
                }
            });
        }
        for session in to_stop {
            session.stop("idle_timeout");
        }
    }

    /// Closes every active VNC session (when the plugin turns off).
    pub(crate) fn close_all(&self) {
        let drained: Vec<Arc<LiveSession>> = {
            let mut map = lock(&self.sessions);
            map.drain().map(|(_, s)| s).collect()
        };
        for session in drained {
            session.stop("plugin_off");
        }
    }

    fn get_live(&self, key: &SessionKey) -> Option<Arc<LiveSession>> {
        let map = lock(&self.sessions);
        let session = map.get(key)?;
        let alive = lock(&session.shared).state != VncSessionState::Closed;
        alive.then(|| Arc::clone(session))
    }

    async fn finish_agent_input(
        &self,
        session: &Arc<LiveSession>,
        before_hash: [u8; 32],
        delay_ms: Option<u64>,
        wait_for_change_ms: Option<u64>,
        take_screenshot: bool,
    ) -> Result<Option<VncScreenshot>, ProxmoxError> {
        self.finish_agent_action(
            session,
            before_hash,
            before_hash,
            DEFAULT_POST_ACTION_WAIT,
            delay_ms,
            wait_for_change_ms,
            take_screenshot,
        )
        .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "post-keyboard wait coordinates pre-input hash, submit baseline, delay, change timeout and screenshot flag"
    )]
    async fn finish_agent_keyboard(
        &self,
        session: &Arc<LiveSession>,
        before_hash: [u8; 32],
        wait_base_hash: [u8; 32],
        ends_with_submit: bool,
        delay_ms: Option<u64>,
        wait_for_change_ms: Option<u64>,
        take_screenshot: bool,
    ) -> Result<Option<VncScreenshot>, ProxmoxError> {
        let default_wait = if ends_with_submit {
            DEFAULT_POST_ACTION_WAIT
        } else {
            POST_TYPING_ECHO_WAIT
        };
        self.finish_agent_action(
            session,
            before_hash,
            wait_base_hash,
            default_wait,
            delay_ms,
            wait_for_change_ms,
            take_screenshot,
        )
        .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "shared post-action delay, change wait, quiet settle and screenshot encoding"
    )]
    async fn finish_agent_action(
        &self,
        session: &Arc<LiveSession>,
        before_hash: [u8; 32],
        wait_base_hash: [u8; 32],
        default_wait: Duration,
        delay_ms: Option<u64>,
        wait_for_change_ms: Option<u64>,
        take_screenshot: bool,
    ) -> Result<Option<VncScreenshot>, ProxmoxError> {
        let has_delay = delay_ms.is_some_and(|ms| ms > 0);
        if let Some(ms) = delay_ms.filter(|&ms| ms > 0) {
            let capped = ms.min(MAX_SCREENSHOT_DELAY_MS);
            self.sleep_while_agent_controls(session, Duration::from_millis(capped))
                .await?;
            session.touch();
        }

        let (w, h, post_delay_hash) = {
            let guard = lock(&session.shared);
            (
                guard.framebuffer.width,
                guard.framebuffer.height,
                guard.framebuffer.hash,
            )
        };
        drop(
            session
                .writer
                .send_framebuffer_update_request(true, 0, 0, w, h)
                .await,
        );

        let wait_budget = match wait_for_change_ms {
            Some(0) => None,
            Some(ms) => Some(Duration::from_millis(ms.min(MAX_WAIT_FOR_CHANGE_MS))),
            None if take_screenshot && !has_delay => Some(default_wait),
            None => None,
        };
        if let Some(budget) = wait_budget {
            let base = if has_delay {
                post_delay_hash
            } else {
                wait_base_hash
            };
            let updated = self.wait_for_hash_change(session, base, budget).await;
            if updated {
                let (quiet, cap) = settle_windows(wait_for_change_ms);
                self.settle_framebuffer(session, quiet, cap).await;
            }
        }

        if !take_screenshot {
            return Ok(None);
        }

        session.ensure_agent_control()?;
        let (png_bytes, out_w, out_h, seq, changed) = {
            let guard = lock(&session.shared);
            let (bytes, out_w, out_h) = guard.framebuffer.encode_tool_screenshot()?;
            (
                bytes,
                out_w,
                out_h,
                guard.framebuffer.seq,
                guard.framebuffer.hash != before_hash,
            )
        };
        Ok(Some(VncScreenshot {
            png_bytes,
            width: out_w,
            height: out_h,
            changed,
            seq,
        }))
    }

    /// Sleeps unconditionally for `duration`, waking immediately if the owner
    /// takes over control or the VNC session disconnects.
    async fn sleep_while_agent_controls(
        &self,
        session: &Arc<LiveSession>,
        duration: Duration,
    ) -> Result<(), ProxmoxError> {
        let deadline = Instant::now() + duration;
        loop {
            let notified = session.frame_notify.notified();
            session.ensure_agent_control()?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(());
            }
            if tokio::time::timeout(remaining, notified).await.is_err() {
                session.ensure_agent_control()?;
                return Ok(());
            }
        }
    }

    /// Waits until framebuffer updates go quiet for `quiet_window` (up to `max_total`),
    /// allowing multi-character console echoes to finish painting before capture.
    async fn settle_framebuffer(
        &self,
        session: &Arc<LiveSession>,
        quiet_window: Duration,
        max_total: Duration,
    ) {
        let cap = Instant::now() + max_total;
        loop {
            let remaining = cap.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return;
            }
            let current_seq = {
                let guard = lock(&session.shared);
                if guard.state == VncSessionState::Closed || guard.control != VncControlOwner::Agent
                {
                    return;
                }
                guard.framebuffer.seq
            };
            let step = remaining.min(quiet_window);
            if !self.wait_for_seq(session, current_seq + 1, step).await {
                return;
            }
        }
    }

    async fn wait_for_seq(
        &self,
        session: &Arc<LiveSession>,
        target_seq: u64,
        timeout: Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let notified = session.frame_notify.notified();
            {
                let guard = lock(&session.shared);
                if guard.framebuffer.seq >= target_seq {
                    return true;
                }
                if guard.state == VncSessionState::Closed {
                    return false;
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            if tokio::time::timeout(remaining, notified).await.is_err() {
                return false;
            }
        }
    }

    async fn wait_for_hash_change(
        &self,
        session: &Arc<LiveSession>,
        initial_hash: [u8; 32],
        timeout: Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let notified = session.frame_notify.notified();
            {
                let guard = lock(&session.shared);
                if guard.framebuffer.hash != initial_hash {
                    return true;
                }
                if guard.state == VncSessionState::Closed || guard.control != VncControlOwner::Agent
                {
                    return false;
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            if tokio::time::timeout(remaining, notified).await.is_err() {
                return false;
            }
        }
    }
}

struct SubscriberGuard {
    counter: Arc<AtomicUsize>,
}

impl SubscriberGuard {
    fn new(counter: Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self { counter }
    }
}

impl Drop for SubscriberGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

struct StreamState {
    session: Arc<LiveSession>,
    _guard: SubscriberGuard,
    sent_initial: bool,
    last_seq: u64,
    last_status_version: u64,
    ended: bool,
}

async fn next_stream_update(mut state: StreamState) -> Option<(VncSessionUpdate, StreamState)> {
    if state.ended {
        return None;
    }

    // 1. Always emit an immediate full Snapshot upon subscribing.
    if !state.sent_initial {
        state.sent_initial = true;
        let (summary, seq, status_version, png_base64) = {
            let mut guard = lock(&state.session.shared);
            guard.stream_dirty = None;
            guard.stream_needs_snapshot = false;
            let full_rect = Rect {
                x: 0,
                y: 0,
                width: guard.framebuffer.width,
                height: guard.framebuffer.height,
            };
            let png = guard
                .framebuffer
                .encode_png_rect(full_rect)
                .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
                .unwrap_or_default();
            (
                VncSessionSummary {
                    conversation_id: state.session.key.conversation_id.clone(),
                    server_id: state.session.key.server_id.clone(),
                    server_name: guard.server_name.clone(),
                    node: guard.node.clone(),
                    vmid: state.session.key.vmid,
                    vm_name: guard.vm_name.clone(),
                    control: guard.control,
                    state: guard.state,
                    width: guard.framebuffer.width,
                    height: guard.framebuffer.height,
                },
                guard.framebuffer.seq,
                guard.status_version,
                png,
            )
        };
        state.last_seq = seq;
        state.last_status_version = status_version;
        return Some((
            VncSessionUpdate::Snapshot {
                session: summary,
                png_base64,
                seq: wire_seq(seq),
            },
            state,
        ));
    }

    // 2. Wait for next frame/status notification, throttled to ~20fps for frame patches.
    let notify = Arc::clone(&state.session.frame_notify);
    loop {
        let notified = notify.notified();
        let next = poll_stream_delta(&state);
        if let Some((update, new_seq, new_status_ver, closed)) = next {
            drop(notified);
            state.last_seq = new_seq;
            state.last_status_version = new_status_ver;
            state.ended = closed;
            return Some((update, state));
        }

        notified.await;
        // Coalesce bursty framebuffer rectangles across one 50ms frame window.
        let is_closed = lock(&state.session.shared).state == VncSessionState::Closed;
        if !is_closed {
            tokio::time::sleep(LIVE_STREAM_INTERVAL).await;
        }
    }
}

fn poll_stream_delta(state: &StreamState) -> Option<(VncSessionUpdate, u64, u64, bool)> {
    let mut guard = lock(&state.session.shared);
    if guard.state == VncSessionState::Closed {
        let reason = guard
            .closed_reason
            .clone()
            .unwrap_or_else(|| "closed".to_owned());
        return Some((
            VncSessionUpdate::Closed { reason },
            guard.framebuffer.seq,
            guard.status_version,
            true,
        ));
    }

    let summary = VncSessionSummary {
        conversation_id: state.session.key.conversation_id.clone(),
        server_id: state.session.key.server_id.clone(),
        server_name: guard.server_name.clone(),
        node: guard.node.clone(),
        vmid: state.session.key.vmid,
        vm_name: guard.vm_name.clone(),
        control: guard.control,
        state: guard.state,
        width: guard.framebuffer.width,
        height: guard.framebuffer.height,
    };

    if guard.stream_needs_snapshot {
        guard.stream_needs_snapshot = false;
        guard.stream_dirty = None;
        let full_rect = Rect {
            x: 0,
            y: 0,
            width: guard.framebuffer.width,
            height: guard.framebuffer.height,
        };
        let png_base64 = guard
            .framebuffer
            .encode_png_rect(full_rect)
            .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
            .unwrap_or_default();
        return Some((
            VncSessionUpdate::Snapshot {
                session: summary,
                png_base64,
                seq: wire_seq(guard.framebuffer.seq),
            },
            guard.framebuffer.seq,
            guard.status_version,
            false,
        ));
    }

    if guard.status_version != state.last_status_version {
        return Some((
            VncSessionUpdate::Status { session: summary },
            state.last_seq,
            guard.status_version,
            false,
        ));
    }

    if let Some(dirty) = guard.stream_dirty.take()
        && dirty.width > 0
        && dirty.height > 0
        && let Ok(png_bytes) = guard.framebuffer.encode_png_rect(dirty)
    {
        let png_base64 = base64::engine::general_purpose::STANDARD.encode(png_bytes);
        return Some((
            VncSessionUpdate::Patch {
                x: dirty.x,
                y: dirty.y,
                width: dirty.width,
                height: dirty.height,
                png_base64,
                seq: wire_seq(guard.framebuffer.seq),
            },
            guard.framebuffer.seq,
            guard.status_version,
            false,
        ));
    }

    None
}

fn wire_seq(seq: u64) -> u32 {
    u32::try_from(seq % (u64::from(u32::MAX) + 1)).unwrap_or(0)
}

async fn open_rfb_connection(
    server: &ServerRecord,
    node: &str,
    vmid: u32,
    api: &ProxmoxApiClient,
    password: &Secret,
) -> Result<
    (
        crate::rfb::RfbReader<
            tokio::io::ReadHalf<TlsStream<TcpStream>>,
            WriteHalf<TlsStream<TcpStream>>,
        >,
        RfbWriter<WriteHalf<TlsStream<TcpStream>>>,
        Framebuffer,
    ),
    ProxmoxError,
> {
    let cookie_ticket = api.current_cookie_ticket(password).await?;
    let proxy = api.create_vnc_proxy(node, vmid, password).await?;
    let tls_config = build_vnc_tls_config(server)?;
    let tls_stream =
        connect_tls(&server.host, server.port, tls_config, VNC_CONNECT_TIMEOUT).await?;

    let host_header = if server.host.contains(':') {
        format!("[{}]:{}", server.host, server.port)
    } else {
        format!("{}:{}", server.host, server.port)
    };
    let encoded_ticket = encode_vnc_ticket(proxy.ticket.as_str());
    let ws_path = format!(
        "/api2/json/nodes/{node}/qemu/{vmid}/vncwebsocket?port={}&vncticket={encoded_ticket}",
        proxy.port
    );
    let cookie_header = format!("PVEAuthCookie={}", cookie_ticket.as_str());
    let ws = tokio::time::timeout(
        VNC_CONNECT_TIMEOUT,
        WsByteStream::connect(tls_stream, &host_header, &ws_path, &cookie_header),
    )
    .await
    .map_err(|_elapsed| ProxmoxError::Timeout)?
    .map_err(|error| describe_vnc_connect_error(&error.to_string()))?;

    let (rfb_client, framebuffer) = tokio::time::timeout(
        VNC_CONNECT_TIMEOUT,
        RfbClient::handshake(ws, &proxy.password),
    )
    .await
    .map_err(|_elapsed| ProxmoxError::Timeout)?
    .map_err(|error| match error {
        ProxmoxError::VncFailed { detail } => describe_vnc_connect_error(&detail),
        other => other,
    })?;
    let (reader, writer) = rfb_client.into_split();
    Ok((reader, writer, framebuffer))
}

fn describe_vnc_connect_error(detail: &str) -> ProxmoxError {
    let lower = detail.to_ascii_lowercase();
    let looks_like_drop = lower.contains("early eof")
        || lower.contains("unexpected eof")
        || lower.contains("connection reset")
        || lower.contains("broken pipe")
        || lower.contains("closed");
    let message = if looks_like_drop {
        format!(
            "VNC console connection closed unexpectedly (the VM may have powered off or rebooted): {detail}"
        )
    } else {
        detail.to_owned()
    };
    ProxmoxError::VncFailed { detail: message }
}

#[expect(
    clippy::too_many_arguments,
    reason = "background RFB reader loop takes ownership of split halves and shared handles"
)]
fn spawn_rfb_reader_loop(
    sessions_map: Arc<Mutex<HashMap<SessionKey, Arc<LiveSession>>>>,
    task_key: SessionKey,
    task_shared: Arc<Mutex<SessionShared>>,
    task_notify: Arc<Notify>,
    task_subs: Arc<AtomicUsize>,
    mut rfb_reader: crate::rfb::RfbReader<
        tokio::io::ReadHalf<TlsStream<TcpStream>>,
        WriteHalf<TlsStream<TcpStream>>,
    >,
    task_writer: RfbWriter<WriteHalf<TlsStream<TcpStream>>>,
    mut shutdown_rx: oneshot::Receiver<String>,
) {
    tokio::spawn(async move {
        let mut local_fb = lock(&task_shared).framebuffer.clone();
        let close_reason = loop {
            tokio::select! {
                biased;
                reason = &mut shutdown_rx => {
                    break reason.unwrap_or_else(|_| "closed".to_owned());
                }
                event = rfb_reader.read_next_event(&mut local_fb) => {
                    match event {
                        Ok(RfbEvent::Updated { dirty, resized }) => {
                            {
                                let mut guard = lock(&task_shared);
                                guard.framebuffer = local_fb.clone();
                                if task_subs.load(Ordering::Relaxed) > 0 {
                                    if resized {
                                        guard.stream_needs_snapshot = true;
                                        guard.stream_dirty = None;
                                    } else {
                                        guard.stream_dirty = Some(match guard.stream_dirty {
                                            Some(existing) => existing.union(dirty),
                                            None => dirty,
                                        });
                                    }
                                }
                            }
                            task_notify.notify_waiters();
                            drop(
                                task_writer
                                    .send_framebuffer_update_request(
                                        true,
                                        0,
                                        0,
                                        local_fb.width,
                                        local_fb.height,
                                    )
                                    .await,
                            );
                        }
                        Ok(RfbEvent::Other) => {}
                        Err(error) => {
                            tracing::debug!(
                                vmid = task_key.vmid,
                                %error,
                                "VNC RFB stream ended"
                            );
                            break "disconnected".to_owned();
                        }
                    }
                }
            }
        };

        task_writer.close().await;
        {
            let mut guard = lock(&task_shared);
            guard.state = VncSessionState::Closed;
            if guard.closed_reason.is_none() {
                guard.closed_reason = Some(close_reason);
            }
            guard.status_version = guard.status_version.saturating_add(1);
        }
        task_notify.notify_waiters();
        lock(&sessions_map).remove(&task_key);
    });
}

async fn send_click(
    writer: &RfbWriter<WriteHalf<TlsStream<TcpStream>>>,
    x: u16,
    y: u16,
    mask: u8,
) -> Result<(), ProxmoxError> {
    writer.send_pointer_event(x, y, mask).await?;
    tokio::time::sleep(INPUT_PULSE_DELAY).await;
    writer.send_pointer_event(x, y, 0).await
}

async fn send_keystroke(
    writer: &RfbWriter<WriteHalf<TlsStream<TcpStream>>>,
    stroke: &KeyStroke,
) -> Result<(), ProxmoxError> {
    for &modifier in &stroke.modifiers {
        writer.send_key_event(modifier, true).await?;
    }
    if !stroke.modifiers.is_empty() {
        tokio::time::sleep(MODIFIER_SETTLE_DELAY).await;
    }
    writer.send_key_event(stroke.key, true).await?;
    tokio::time::sleep(INPUT_PULSE_DELAY).await;
    writer.send_key_event(stroke.key, false).await?;
    if !stroke.modifiers.is_empty() {
        tokio::time::sleep(MODIFIER_SETTLE_DELAY).await;
        for &modifier in stroke.modifiers.iter().rev() {
            writer.send_key_event(modifier, false).await?;
        }
    }
    Ok(())
}

fn button_mask(button: &str) -> Result<u8, ProxmoxError> {
    match button.to_ascii_lowercase().as_str() {
        "left" => Ok(0x01),
        "middle" => Ok(0x02),
        "right" => Ok(0x04),
        other => Err(ProxmoxError::InvalidRequest {
            detail: format!("unknown mouse button {other:?}"),
        }),
    }
}

fn settle_windows(wait_for_change_ms: Option<u64>) -> (Duration, Duration) {
    if wait_for_change_ms.is_some_and(|ms| ms >= LONG_WAIT_THRESHOLD_MS) {
        (LONG_INPUT_QUIET_DELAY, LONG_INPUT_SETTLE_CAP)
    } else {
        (POST_INPUT_QUIET_DELAY, POST_INPUT_SETTLE_CAP)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::{
        LONG_INPUT_QUIET_DELAY, LONG_INPUT_SETTLE_CAP, LONG_WAIT_THRESHOLD_MS,
        POST_INPUT_QUIET_DELAY, POST_INPUT_SETTLE_CAP, describe_vnc_connect_error, settle_windows,
    };
    use crate::model::ProxmoxError;

    #[test]
    fn annotates_vnc_eof_errors_with_reboot_hint() {
        let ProxmoxError::VncFailed { detail } =
            describe_vnc_connect_error("WebSocket I/O error: early eof")
        else {
            panic!("expected VncFailed");
        };
        assert!(detail.contains("powered off or rebooted"));
        assert!(detail.contains("early eof"));

        let ProxmoxError::VncFailed { detail } =
            describe_vnc_connect_error("VNC authentication failed")
        else {
            panic!("expected VncFailed");
        };
        assert_eq!(detail, "VNC authentication failed");
    }

    #[test]
    fn settle_windows_widen_for_long_explicit_waits() {
        assert_eq!(
            settle_windows(None),
            (POST_INPUT_QUIET_DELAY, POST_INPUT_SETTLE_CAP)
        );
        assert_eq!(
            settle_windows(Some(500)),
            (POST_INPUT_QUIET_DELAY, POST_INPUT_SETTLE_CAP)
        );
        assert_eq!(
            settle_windows(Some(LONG_WAIT_THRESHOLD_MS - 1)),
            (POST_INPUT_QUIET_DELAY, POST_INPUT_SETTLE_CAP)
        );
        assert_eq!(
            settle_windows(Some(LONG_WAIT_THRESHOLD_MS)),
            (LONG_INPUT_QUIET_DELAY, LONG_INPUT_SETTLE_CAP)
        );
        assert_eq!(
            settle_windows(Some(5_000)),
            (LONG_INPUT_QUIET_DELAY, LONG_INPUT_SETTLE_CAP)
        );
    }
}
