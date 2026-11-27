# Changelog of the proxmox plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Fixed

- VNC WebSocket connections now match Proxmox REST and browser handshakes behind reverse proxies by omitting `:443` from the `Host` header, advertising `http/1.1` ALPN, sending `Origin`, `User-Agent`, `Pragma`, and `Cache-Control` headers, preserving origin-scoped reverse-proxy session cookies across `vncproxy` and `vncwebsocket`, and reporting scrubbed gateway diagnostics on non-101 responses.
