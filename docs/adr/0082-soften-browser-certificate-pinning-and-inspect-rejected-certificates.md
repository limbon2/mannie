# ADR-0082: Soften Browser certificate pinning and inspect rejected certificates

|          |                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D76)                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Date     | 2026-11-26                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Related  | [ADR-0002](0002-grow-through-extension-points.md), [ADR-0064](0064-give-agents-an-isolated-browser-beside-the-conversation.md), [ADR-0065](0065-let-browser-access-follow-the-conversation.md), [ADR-0066](0066-let-the-owner-trust-self-signed-browser-certificates.md), [ADR-0067](0067-manage-browser-trust-and-credentials-in-settings.md), [Browser](../../plugins/browser/docs/README.md), [Security](../architecture/security.md) |

## Context

- [ADR-0066](0066-let-the-owner-trust-self-signed-browser-certificates.md), decision 3, required Browser's native certificate callback to accept only WebView2's `CERTIFICATE_IS_INVALID` status and to independently verify that a pinned leaf certificate is strictly self-signed (`Issuer == Subject` and signed by its own key), passes Windows `CertGetCertificateChain` with the leaf as sole trust anchor, includes a matching SAN IP or DNS entry under `CERT_CHAIN_POLICY_SSL`, uses RSA or ECDSA with SHA-256 or stronger under `szOID_CERT_STRONG_SIGN_OS_1`, carries `ServerAuth` EKU, and stays valid beyond a five-second expiry timer that terminates the session.
- In real local and internal setups, those checks rejected valid owner-pinned certificates even when the exact SHA-256 fingerprint matched:
  - Private and local CA certificates (such as Proxmox VE cluster CA certificates, Caddy `tls internal`, `mkcert`, `step-ca`, pfSense, OPNsense, TrueNAS and Synology) have `Issuer != Subject` and are signed by their local CA rather than by themselves.
  - Servers opened by LAN IP, Tailscale IP, SSH tunnel (`localhost`) or alias not present in SAN, and older appliances with CN-only certificates, trigger `CERTIFICATE_COMMON_NAME_IS_INCORRECT` or fail SAN checks.
  - Modern Ed25519 and Ed448 certificates, 1024-bit appliance keys, and expired dev or BMC certificates were rejected or ended active sessions abruptly.
  - Storing only one fingerprint per hostname blocked trusting separate HTTPS services on different ports of the same host.
  - WebSocket (`wss://`) upgrades were excluded by checking only `https`.
  - Rejections emitted no diagnostic log and showed no presented certificate details, leaving the owner unable to tell why a pinned host failed.
- Verified on 2026-11-26:
  - During the TLS handshake, WebView2 verifies that the remote server possesses the private key matching the presented leaf certificate's public key before `ServerCertificateErrorDetected` fires. Pinning the exact 256-bit SHA-256 digest of the leaf X.509 DER commits to that exact public key, subject, issuer, SANs, validity and extensions.
  - `ICoreWebView2ServerCertificateErrorDetectedEventArgs` supplies `ErrorStatus`, `RequestUri` and `ServerCertificate().ToPemEncoding()` synchronously during the callback without extra network requests or waiting inside WebView2.

## Decision

1. Supersede [ADR-0066](0066-let-the-owner-trust-self-signed-browser-certificates.md), decision 3, and refine [ADR-0067](0067-manage-browser-trust-and-credentials-in-settings.md), decision 3:
   - Cancel server certificate errors by default.
   - Allow a request when its scheme is `https` or `wss`, WebView2 reports `CERTIFICATE_IS_INVALID`, `CERTIFICATE_COMMON_NAME_IS_INCORRECT` or `CERTIFICATE_EXPIRED`, the leaf certificate DER is at most 64 KiB and parses as a structurally valid X.509 certificate, and the canonical `(hostname, leaf DER SHA-256)` pair is in the owner's saved pins.
   - Keep `CERTIFICATE_REVOKED` cancelled and ineligible for pinning.
   - Drop the independent `Issuer == Subject`, self-signature, exclusive-root chain, SAN/hostname, `ServerAuth` EKU, signature algorithm, key-strength and validity-window rejections for owner-pinned leaves.
   - Remove the five-second expiry cutoff and active-session expiry timer. Retain `BrowserError::CertificateExpired` in the SDK contract as an unused variant per [ADR-0002](0002-grow-through-extension-points.md).
2. Allow multiple saved certificate pins per hostname (`PRIMARY KEY (hostname, fingerprint)` via `browser` migration 4), superseding the single-pin-per-hostname restriction in [ADR-0066](0066-let-the-owner-trust-self-signed-browser-certificates.md), decision 4:
   - Saving a new `(hostname, fingerprint)` pair adds it beside existing pins for that hostname and journals `"save"`. Saving an identical pair is idempotent. Retain `BrowserError::CertificateSavedConflict` in the SDK contract as an unused variant per [ADR-0002](0002-grow-through-extension-points.md).
   - Forgetting an exact `(hostname, fingerprint)` pair removes only that pin, journals `"forget"`, and terminates active Browser sessions that allowed that exact pair before returning success.
   - Accept pasted HTTP, HTTPS or WSS URLs and `host:port` values in the hostname field (extracting the canonical bare host while refusing userinfo, wildcards and non-web schemes), and accept SHA-256 fingerprints with optional `SHA256 Fingerprint=` / `sha256:` prefixes and colon, space or hyphen separators.
3. Add owner-only in-pane certificate inspection and trust confirmation, superseding the Settings-only creation requirement in [ADR-0066](0066-let-the-owner-trust-self-signed-browser-certificates.md), decision 2, and [ADR-0067](0067-manage-browser-trust-and-credentials-in-settings.md), decision 3:
   - On a rejected top-level navigation (matching the tab's active navigation URL), parse the presented leaf DER with a bounded, zero-`unsafe` ASN.1 DER parser and carry sanitized metadata (`BrowserRejectedCertificate`: hostname, SHA-256 fingerprint, subject, issuer, SANs, validity window, expired flag, self-signed flag, error reason and query-stripped URL) on `BrowserTab` in owner-only `BrowserStatus`.
   - Background and sub-resource certificate errors remain silently cancelled and never populate `rejected_certificate` or open a prompt.
   - `BrowserRejectedCertificate` never reaches model tool outputs.
   - The Browser pane's certificate error banner offers **Inspect certificate**, which opens an owner confirmation dialog (hiding the native page surface while open) showing the presented certificate details, any expired or revoked warning, and a **Save trust** / **Save trust and retry** action that writes through the same journaled `browser_save_certificate` path as **Settings > Browser**.
   - Saving trust never grants agent site access or transfers control; per-origin agent approvals remain separate ([ADR-0065](0065-let-browser-access-follow-the-conversation.md)).
4. Log every certificate rejection and pinned allowance with structured fields (canonical hostname, WebView2 error reason, rejection cause, presented leaf SHA-256 fingerprint, subject, issuer and expiry flag) so certificate failures are directly diagnosable from Mannie's logs.
5. Add `BrowserError::NoPageLoaded` (`no_page_loaded`) to the SDK contract so agent requests targeting a blank or failed tab report `certificate_rejected` or `no_page_loaded` instead of `invalid_browser_input`. Treat hide-only (`visible: false`) `browser_place` calls on missing sessions or tabs without a lease as a safe no-op while preserving strict lease checks for visible placements (T-110).
6. Record threat `T-135` in [security](../architecture/security.md).

Earlier decisions that this ADR changes:

| Earlier decision                                                                                  | Change                                                                                                                                                                            |
| ------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [ADR-0066](0066-let-the-owner-trust-self-signed-browser-certificates.md), decisions 2, 3, 4 and 5 | Superseded in part: drop self-signed/SAN/algorithm/expiry checks on pinned leaves, remove the 5-second expiry timer, allow multiple pins per hostname, and add in-pane inspection |
| [ADR-0067](0067-manage-browser-trust-and-credentials-in-settings.md), decision 3                  | Refined: Settings remains the catalog for viewing, adding and forgetting pins, while the Browser pane also offers owner-only inspection and trust confirmation on rejected sites  |

## Consequences

- Positive:
  - Private-CA-signed sites (Proxmox VE, Caddy, `mkcert`, local CAs), IP-accessed servers, CN-only appliances, Ed25519 certificates, expired local certificates and multi-port services on one host work once pinned.
  - WebSocket (`wss://`) connections on trusted hosts succeed.
  - The owner can inspect the exact presented certificate and confirm its fingerprint directly from the Browser pane without switching to external tools.
  - Every rejection is explained in Mannie's logs and pane banner.
  - Deleting the WinCrypt chain engine removes roughly 350 lines of `unsafe` Win32 code in favor of a pure-Rust ASN.1 DER metadata parser testable on all platforms.
- Negative:
  - Trusting an expired pinned certificate remains active until forgotten by the owner rather than terminating sessions automatically.
  - `BrowserTab` carries an optional `rejected_certificate` payload when a top-level navigation fails TLS verification.
- Risks:
  - An owner clicking through the in-pane inspection dialog without checking the fingerprint on an untrusted network could pin a man-in-the-middle certificate. The dialog displays the full SHA-256 fingerprint, subject, issuer, SANs, validity dates and error reason clearly before confirmation, keeps revoked certificates blocked from pinning, and still requires explicit per-origin agent approval separately (T-135).

## Alternatives considered

| Option                                         | Why not chosen                                                                                                                                  |
| ---------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| Keep strict `Issuer == Subject` and SAN checks | Rejects Proxmox VE, Caddy `tls internal`, `mkcert`, local CAs, CN-only appliances and servers opened by IP even when pinned                     |
| Add a host-level "trust any certificate" mode  | Weakens protection against unexpected certificate substitution; exact SHA-256 pinning with in-pane inspection gives both safety and convenience |
| Keep Settings-only entry without inspection    | Forces the owner to inspect certificates in external tools and leaves certificate rejections opaque                                             |
| Keep the five-second expiry session hard-kill  | Abruptly terminates active Browser sessions on expired or short-lived dev and appliance certificates the owner explicitly pinned                |

## Validation

Extend [V13](../roadmap/milestones.md#4-validations) on Windows with tests covering:

- pure-Rust DER parsing of self-signed, private-CA-signed (`Issuer != Subject`), CN-only, Ed25519, weak-key, expired and malformed certificates;
- forgiving hostname (URLs, `host:port`, `wss://`) and fingerprint (prefixed, colon, space, hyphen) parsing;
- SQLite `browser` v3-to-v4 migration and multi-pin storage/forgetting;
- end-to-end WebView2 flows for private-CA-signed, hostname-mismatched and expired certificates via the in-pane Inspect certificate dialog, multiple certificates on different ports of `127.0.0.1`, continued sub-resource isolation, rotation rejection, restart persistence, revocation teardown, and light/dark axe accessibility.
