# Browser changelog

Changes to the Browser plugin. Releases collect its entries in the [Mannie changelog](../../CHANGELOG.md).

## Unreleased

### Added

- Add a self-management provider exposing saved site credential metadata and certificate pins with chat-driven actions to add or remove credentials and certificate pins ([ADR-0081](../../docs/adr/0081-let-agents-inspect-and-configure-mannie.md)).
- Add an owner-only **Inspect certificate** dialog on Browser certificate error banners showing presented certificate details with one-click **Save trust and retry**, and support multiple saved certificate pins per hostname ([ADR-0082](../../docs/adr/0082-soften-browser-certificate-pinning-and-inspect-rejected-certificates.md)).

### Changed

- Soften Browser SHA-256 leaf certificate pinning to accept private-CA-signed, hostname-mismatched, CN-only, Ed25519 and expired certificates over `https` and `wss`, remove the five-second active-session expiry hard-kill, accept pasted URLs and formatted fingerprints, add structured rejection logs, return `certificate_rejected` or `no_page_loaded` on unloaded agent tabs, and treat hide-only `browser_place` calls on ended sessions as a no-op ([ADR-0082](../../docs/adr/0082-soften-browser-certificate-pinning-and-inspect-rejected-certificates.md)).
