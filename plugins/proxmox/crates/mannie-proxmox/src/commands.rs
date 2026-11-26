//! IPC command and channel specifications and handlers for the Proxmox plugin
//! (ADR-0034, decision 8; ADR-0080).

use futures_util::{StreamExt as _, stream};
use mannie_plugin_sdk::ipc::{ChannelSpec, CommandSpec, Handlers, Interface, PluginStream};
use tokio::sync::watch;

use crate::model::{
    AddServer, CertificateInfo, DEFAULT_PORT, DiscoverableVmView, MAX_HOST_CHARS, MAX_NAME_CHARS,
    MAX_NOTE_CHARS, MAX_USERNAME_CHARS, PinCertificate, ProxmoxError, ServerListUpdate, ServerRef,
    ServerState, ServerView, SetAllowedVms, SetVncControl, TestConnectionResult, UpdateServer,
    VncConversationRef, VncKeyboardInput, VncPointerInput, VncSessionSummary, VncSessionTarget,
    VncSessionUpdate,
};
use crate::state::ProxmoxManager;

/// Adds a Proxmox VE server with zero allowed VMs by default.
#[derive(Debug)]
pub struct AddServerCommand;
impl CommandSpec for AddServerCommand {
    const NAME: &'static str = "add_server";
    const DOCS: &'static str = "Registers a Proxmox VE server with zero allowed VMs by default and stores its password in the OS credential store.";
    type Input = AddServer;
    type Output = ServerView;
    type Error = ProxmoxError;
}

/// Updates an existing Proxmox server (host/port edits clear pin & allowlist).
#[derive(Debug)]
pub struct UpdateServerCommand;
impl CommandSpec for UpdateServerCommand {
    const NAME: &'static str = "update_server";
    const DOCS: &'static str = "Updates a Proxmox VE server. Changing host or port clears its pinned certificate and allowed VM list.";
    type Input = UpdateServer;
    type Output = ServerView;
    type Error = ProxmoxError;
}

/// Removes a Proxmox server, its password from the OS keyring, and active sessions.
#[derive(Debug)]
pub struct RemoveServerCommand;
impl CommandSpec for RemoveServerCommand {
    const NAME: &'static str = "remove_server";
    const DOCS: &'static str = "Removes a Proxmox VE server, deletes its stored password, and closes its live VNC sessions.";
    type Input = ServerRef;
    type Output = ();
    type Error = ProxmoxError;
}

/// Performs a TLS-only certificate inspection without sending credentials.
#[derive(Debug)]
pub struct InspectCertificateCommand;
impl CommandSpec for InspectCertificateCommand {
    const NAME: &'static str = "inspect_certificate";
    const DOCS: &'static str = "Inspects a server's presented leaf TLS certificate over a TLS-only handshake without sending credentials.";
    type Input = ServerRef;
    type Output = CertificateInfo;
    type Error = ProxmoxError;
}

/// Pins a verified leaf certificate SHA-256 fingerprint on a server.
#[derive(Debug)]
pub struct PinCertificateCommand;
impl CommandSpec for PinCertificateCommand {
    const NAME: &'static str = "pin_certificate";
    const DOCS: &'static str = "Pins the server's leaf certificate SHA-256 fingerprint after verifying it matches a live TLS handshake.";
    type Input = PinCertificate;
    type Output = ServerView;
    type Error = ProxmoxError;
}

/// Clears a server's pinned certificate, returning it to OS trust verification.
#[derive(Debug)]
pub struct ClearCertificatePinCommand;
impl CommandSpec for ClearCertificatePinCommand {
    const NAME: &'static str = "clear_certificate_pin";
    const DOCS: &'static str = "Clears a server's pinned leaf certificate so connections verify against the system trust store.";
    type Input = ServerRef;
    type Output = ServerView;
    type Error = ProxmoxError;
}

/// Tests TLS trust, authentication, and QEMU VM listing for a registered server.
#[derive(Debug)]
pub struct TestConnectionCommand;
impl CommandSpec for TestConnectionCommand {
    const NAME: &'static str = "test_connection";
    const DOCS: &'static str =
        "Tests TLS trust, Proxmox ticket login, and QEMU VM visibility for a registered server.";
    type Input = ServerRef;
    type Output = TestConnectionResult;
    type Error = ProxmoxError;
}

/// Lists all QEMU VMs visible to the server's account for the Settings allowlist UI.
#[derive(Debug)]
pub struct DiscoverServerVmsCommand;
impl CommandSpec for DiscoverServerVmsCommand {
    const NAME: &'static str = "discover_server_vms";
    const DOCS: &'static str = "Lists all QEMU VMs visible to the server's Proxmox account so the owner can choose which VMs to allow.";
    type Input = ServerRef;
    type Output = Vec<DiscoverableVmView>;
    type Error = ProxmoxError;
}

/// Saves the owner's explicit per-server VM allowlist.
#[derive(Debug)]
pub struct SetAllowedVmsCommand;
impl CommandSpec for SetAllowedVmsCommand {
    const NAME: &'static str = "set_allowed_vms";
    const DOCS: &'static str = "Replaces the owner's explicit per-server QEMU VM allowlist and closes live VNC sessions for unchecked VMs.";
    type Input = SetAllowedVms;
    type Output = ServerView;
    type Error = ProxmoxError;
}

/// Lists live VNC sessions open for a conversation.
#[derive(Debug)]
pub struct ListVncSessionsCommand;
impl CommandSpec for ListVncSessionsCommand {
    const NAME: &'static str = "list_vnc_sessions";
    const DOCS: &'static str =
        "Lists active live VNC console sessions belonging to a conversation.";
    type Input = VncConversationRef;
    type Output = Vec<VncSessionSummary>;
    type Error = ProxmoxError;
}

/// Opens or attaches to a live VNC session for an allowed VM in the side panel.
#[derive(Debug)]
pub struct OpenVncSessionCommand;
impl CommandSpec for OpenVncSessionCommand {
    const NAME: &'static str = "open_vnc_session";
    const DOCS: &'static str =
        "Opens or attaches to a live VNC console session for an allowed QEMU VM.";
    type Input = VncSessionTarget;
    type Output = VncSessionSummary;
    type Error = ProxmoxError;
}

/// Switches live VNC console control between owner takeover and agent mode.
#[derive(Debug)]
pub struct SetVncControlCommand;
impl CommandSpec for SetVncControlCommand {
    const NAME: &'static str = "set_vnc_control";
    const DOCS: &'static str =
        "Switches a live VNC session between owner takeover and agent control.";
    type Input = SetVncControl;
    type Output = VncSessionSummary;
    type Error = ProxmoxError;
}

/// Sends an owner pointer event in the live VNC panel (requires owner takeover).
#[derive(Debug)]
pub struct VncPointerCommand;
impl CommandSpec for VncPointerCommand {
    const NAME: &'static str = "vnc_pointer";
    const DOCS: &'static str =
        "Sends an owner pointer event to a live VNC session while in owner takeover mode.";
    type Input = VncPointerInput;
    type Output = ();
    type Error = ProxmoxError;
}

/// Sends an owner keyboard event in the live VNC panel (requires owner takeover).
#[derive(Debug)]
pub struct VncKeyboardCommand;
impl CommandSpec for VncKeyboardCommand {
    const NAME: &'static str = "vnc_keyboard";
    const DOCS: &'static str =
        "Sends an owner keyboard event to a live VNC session while in owner takeover mode.";
    type Input = VncKeyboardInput;
    type Output = ();
    type Error = ProxmoxError;
}

/// Sends Ctrl+Alt+Delete to the VM from the live VNC toolbar (requires owner takeover).
#[derive(Debug)]
pub struct VncSendCtrlAltDelCommand;
impl CommandSpec for VncSendCtrlAltDelCommand {
    const NAME: &'static str = "vnc_send_ctrl_alt_del";
    const DOCS: &'static str =
        "Sends Ctrl+Alt+Delete to a live VNC session while in owner takeover mode.";
    type Input = VncSessionTarget;
    type Output = ();
    type Error = ProxmoxError;
}

/// Closes a live VNC session on behalf of the owner.
#[derive(Debug)]
pub struct CloseVncSessionCommand;
impl CommandSpec for CloseVncSessionCommand {
    const NAME: &'static str = "close_vnc_session";
    const DOCS: &'static str = "Closes a live VNC session on behalf of the owner.";
    type Input = VncSessionTarget;
    type Output = ();
    type Error = ProxmoxError;
}

/// Streams registered Proxmox servers and their allowed VMs to Settings and selectors.
#[derive(Debug)]
pub struct FollowServersChannel;
impl ChannelSpec for FollowServersChannel {
    const NAME: &'static str = "follow_servers";
    const DOCS: &'static str =
        "Streams registered Proxmox servers and their explicitly allowed VMs.";
    type Input = ();
    type Item = ServerListUpdate;
    type Error = ProxmoxError;
}

/// Streams live PNG snapshots, dirty-rect patches, control changes, and close
/// notices for one VNC session to the side panel.
#[derive(Debug)]
pub struct FollowVncSessionChannel;
impl ChannelSpec for FollowVncSessionChannel {
    const NAME: &'static str = "follow_vnc_session";
    const DOCS: &'static str = "Streams live VNC framebuffer snapshots, dirty-rect PNG patches, control updates, and close events.";
    type Input = VncSessionTarget;
    type Item = VncSessionUpdate;
    type Error = ProxmoxError;
}

/// Declares the plugin's full IPC interface for binding generation and grants.
pub fn interface() -> Interface {
    Interface::new()
        .command::<AddServerCommand>()
        .command::<UpdateServerCommand>()
        .command::<RemoveServerCommand>()
        .command::<InspectCertificateCommand>()
        .command::<PinCertificateCommand>()
        .command::<ClearCertificatePinCommand>()
        .command::<TestConnectionCommand>()
        .command::<DiscoverServerVmsCommand>()
        .command::<SetAllowedVmsCommand>()
        .command::<ListVncSessionsCommand>()
        .command::<OpenVncSessionCommand>()
        .command::<SetVncControlCommand>()
        .command::<VncPointerCommand>()
        .command::<VncKeyboardCommand>()
        .command::<VncSendCtrlAltDelCommand>()
        .command::<CloseVncSessionCommand>()
        .channel::<FollowServersChannel>()
        .channel::<FollowVncSessionChannel>()
        .constant(
            "DEFAULT_PORT",
            "Default Proxmox VE HTTPS API port.",
            u32::from(DEFAULT_PORT),
        )
        .constant(
            "MAX_NAME_CHARS",
            "Maximum characters of a Proxmox server display name.",
            u32::try_from(MAX_NAME_CHARS).unwrap_or(u32::MAX),
        )
        .constant(
            "MAX_HOST_CHARS",
            "Maximum characters of a Proxmox server host or IP.",
            u32::try_from(MAX_HOST_CHARS).unwrap_or(u32::MAX),
        )
        .constant(
            "MAX_USERNAME_CHARS",
            "Maximum characters of a Proxmox username with realm.",
            u32::try_from(MAX_USERNAME_CHARS).unwrap_or(u32::MAX),
        )
        .constant(
            "MAX_NOTE_CHARS",
            "Maximum characters of a Proxmox server note.",
            u32::try_from(MAX_NOTE_CHARS).unwrap_or(u32::MAX),
        )
}

/// Binds IPC handlers to a running `ProxmoxManager`.
pub(crate) fn handlers(manager: &ProxmoxManager) -> Handlers {
    let m_add = manager.clone();
    let m_update = manager.clone();
    let m_remove = manager.clone();
    let m_inspect = manager.clone();
    let m_pin = manager.clone();
    let m_clear_pin = manager.clone();
    let m_test = manager.clone();
    let m_discover = manager.clone();
    let m_allow = manager.clone();
    let m_list_vnc = manager.clone();
    let m_open_vnc = manager.clone();
    let m_set_ctrl = manager.clone();
    let m_ptr = manager.clone();
    let m_kbd = manager.clone();
    let m_cad = manager.clone();
    let m_close_vnc = manager.clone();
    let m_follow_servers = manager.clone();
    let m_follow_vnc = manager.clone();

    Handlers::new()
        .command::<AddServerCommand, _, _>(move |input: AddServer| {
            let mgr = m_add.clone();
            async move { mgr.add_server(input).await.map(|state| state.view()) }
        })
        .command::<UpdateServerCommand, _, _>(move |input: UpdateServer| {
            let mgr = m_update.clone();
            async move { mgr.update_server(input).await.map(|state| state.view()) }
        })
        .command::<RemoveServerCommand, _, _>(move |input: ServerRef| {
            let mgr = m_remove.clone();
            async move { mgr.remove_server(&input.server_id).await }
        })
        .command::<InspectCertificateCommand, _, _>(move |input: ServerRef| {
            let mgr = m_inspect.clone();
            async move { mgr.inspect_certificate(&input.server_id).await }
        })
        .command::<PinCertificateCommand, _, _>(move |input: PinCertificate| {
            let mgr = m_pin.clone();
            async move { mgr.pin_certificate(input).await.map(|state| state.view()) }
        })
        .command::<ClearCertificatePinCommand, _, _>(move |input: ServerRef| {
            let mgr = m_clear_pin.clone();
            async move {
                mgr.clear_certificate_pin(&input.server_id)
                    .await
                    .map(|state| state.view())
            }
        })
        .command::<TestConnectionCommand, _, _>(move |input: ServerRef| {
            let mgr = m_test.clone();
            async move { mgr.test_connection(&input.server_id).await }
        })
        .command::<DiscoverServerVmsCommand, _, _>(move |input: ServerRef| {
            let mgr = m_discover.clone();
            async move { mgr.discover_server_vms(&input.server_id).await }
        })
        .command::<SetAllowedVmsCommand, _, _>(move |input: SetAllowedVms| {
            let mgr = m_allow.clone();
            async move { mgr.set_allowed_vms(input).await.map(|state| state.view()) }
        })
        .command::<ListVncSessionsCommand, _, _>(move |input: VncConversationRef| {
            let mgr = m_list_vnc.clone();
            async move { Ok::<_, ProxmoxError>(mgr.list_vnc_sessions(&input.conversation_id)) }
        })
        .command::<OpenVncSessionCommand, _, _>(move |input: VncSessionTarget| {
            let mgr = m_open_vnc.clone();
            async move { mgr.open_vnc_session_for_owner(input).await }
        })
        .command::<SetVncControlCommand, _, _>(move |input: SetVncControl| {
            let mgr = m_set_ctrl.clone();
            async move { mgr.set_vnc_control(input) }
        })
        .command::<VncPointerCommand, _, _>(move |input: VncPointerInput| {
            let mgr = m_ptr.clone();
            async move { mgr.owner_vnc_pointer(input).await }
        })
        .command::<VncKeyboardCommand, _, _>(move |input: VncKeyboardInput| {
            let mgr = m_kbd.clone();
            async move { mgr.owner_vnc_keyboard(input).await }
        })
        .command::<VncSendCtrlAltDelCommand, _, _>(move |input: VncSessionTarget| {
            let mgr = m_cad.clone();
            async move { mgr.owner_vnc_ctrl_alt_del(input).await }
        })
        .command::<CloseVncSessionCommand, _, _>(move |input: VncSessionTarget| {
            let mgr = m_close_vnc.clone();
            async move {
                mgr.owner_vnc_close(input);
                Ok::<_, ProxmoxError>(())
            }
        })
        .channel::<FollowServersChannel, _, _>(move |()| {
            std::future::ready(Ok::<_, ProxmoxError>(server_list_stream(
                m_follow_servers.follow_servers(),
            )))
        })
        .channel::<FollowVncSessionChannel, _, _>(move |input: VncSessionTarget| {
            let mgr = m_follow_vnc.clone();
            async move { mgr.follow_vnc_session(input) }
        })
}

fn server_list_stream(rx: watch::Receiver<Vec<ServerState>>) -> PluginStream<ServerListUpdate> {
    stream::unfold((rx, true), |(mut rx, first)| async move {
        if !first && rx.changed().await.is_err() {
            return None;
        }

        let servers = rx
            .borrow_and_update()
            .iter()
            .map(ServerState::view)
            .collect();
        Some((ServerListUpdate::Servers { servers }, (rx, false)))
    })
    .boxed()
}
