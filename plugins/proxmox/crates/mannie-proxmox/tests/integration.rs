//! Integration tests for the Proxmox plugin: SDK conformance, lifecycle,
//! TLS inspection and leaf pinning, fail-closed VM allowlist enforcement,
//! live VNC takeover and full OS-install tool workflows (ADR-0080).

#[expect(
    clippy::unwrap_used,
    reason = "integration tests assert invariants directly with unwrap/expect"
)]
mod integration {
    mod allowlist;
    mod plugin;
    mod support;
    mod tls;
    mod tools;
}
