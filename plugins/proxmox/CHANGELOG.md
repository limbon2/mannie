# Changelog of the proxmox plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Added

- Add the Proxmox plugin (`proxmox`) for regular non-admin Proxmox VE user accounts with explicit per-server QEMU VM allowlist pre-configuration, OS-trusted and pinned SHA-256 leaf TLS verification, write-only OS credential storage, auto-renewed ticket sessions, ISO/boot-order/power tools under per-VM `Access::ProxmoxVm` approvals, pure-Rust RFB 3.8 VNC tools with PNG screenshots, and a live VNC canvas side panel with owner takeover ([ADR-0080](../../docs/adr/0080-let-plugins-manage-proxmox-vms-and-vnc-consoles.md)).
