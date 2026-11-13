# ADR-0066: Let the owner trust self-signed Browser certificates

|          |                                                                                                                                                                                              |
| -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D61)                                                                                                                                                                               |
| Date     | 2026-11-13                                                                                                                                                                                   |
| Deciders | Owner                                                                                                                                                                                        |
| Related  | [ADR-0064](0064-give-agents-an-isolated-browser-beside-the-conversation.md), [ADR-0065](0065-let-browser-access-follow-the-conversation.md), [Browser](../../plugins/browser/docs/README.md) |

This revision follows D61. D60 records the original decision to trust a certificate for one session.

## Context

The owner needs Browser to open HTTPS sites that use self-signed server certificates without re-entering the same fingerprint in every session. [ADR-0064](0064-give-agents-an-isolated-browser-beside-the-conversation.md), decision 8, requires the host to cancel server certificate errors. Trusting a private certificate in Windows can make it available to WebView2 and other applications that use the same store.

The [WebView2 certificate error handler](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2_14) runs for page requests and resources loaded by pages. It can allow an error for the request hostname and the presented certificate. WebView2 caches that decision across the tabs in the session, including other ports on the same hostname. Clearing cached decisions leaves existing connections open. Its [certificate error statuses](https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.core.corewebview2weberrorstatus) group several verification failures under an invalid-certificate status. Verified on 2026-11-13.

## Decision

1. Supersede the requirement to cancel all server certificate errors in [ADR-0064](0064-give-agents-an-isolated-browser-beside-the-conversation.md), decision 8. Cancel server certificate errors by default. Let the owner explicitly trust a self-signed server certificate for an HTTPS hostname for the current Browser session or save the trust for Browser on this Mannie profile.
2. The owner enters the hostname and SHA-256 fingerprint of the server certificate in the Browser pane. The pane explains where to get the fingerprint from the site's administrator or server configuration and shows the scope of each choice before the owner confirms. A failed navigation identifies a certificate error and tells the owner where to add the exception. Only the owner can change trust. A website, background request or agent cannot open a confirmation. No request waits in WebView2 while the owner decides. The owner or agent retries through ordinary navigation.
3. The native handler matches the request hostname and the SHA-256 fingerprint of the presented leaf certificate. Before the first allowance, independently verify that the certificate is self-signed and passes HTTPS server certificate checks with that leaf as its only trust anchor, including hostname and validity. WebView2's generic invalid-certificate status alone is insufficient. Cancel the request if verification fails or the runtime reports a different certificate error. Refuse a new allowance if five seconds or less remain before expiry. Once WebView2 has cached an allowance, end the Browser session five seconds before the earliest allowed certificate expires so cached decisions and open connections end with it.
4. Trust is scoped to the hostname and certificate across tabs and HTTPS ports. Mannie stores one saved fingerprint per hostname in its database, shared across Browser sessions and conversations and retained after Mannie restarts. Each session can hold one temporary fingerprint per hostname. The temporary fingerprint takes precedence in that session. Ending a session discards its temporary trust and WebView2's cached allowances. The upgrade keeps earlier confirmations as audit history. Mannie makes no changes to the Windows certificate store.
5. The owner can see and forget saved trust from the Browser pane. Forgetting the exact hostname and fingerprint removes the saved entry and ends any session that allowed that pair or holds it as temporary trust. Mannie closes those sessions and their connections before reporting success. To save trust for a replacement certificate, the owner forgets the old entry and confirms the replacement's fingerprint.
6. Browser site approval still checks each top-level destination by its exact origin, including scheme and port, and governs agent access as in [ADR-0065](0065-let-browser-access-follow-the-conversation.md). Journal the owner's trust changes, and commit a saved trust change with its journal record before publishing it to Browser. A certificate exception grants no site access or control to the agent.

## Consequences

- The owner can use saved trust for local and intranet HTTPS sites in any Browser conversation on the same Mannie profile. Other applications use their usual certificate stores.
- The owner verifies a fingerprint again when the server replaces its certificate. Forgetting saved trust closes the tabs in the affected sessions. The same teardown happens before a certificate an active session allowed expires.
- WebView2 can reuse the allowance for resources loaded by pages and for other ports serving the same certificate at that hostname. Site approval still applies to each top-level origin.

## Alternatives considered

| Option                                  | Reason to decline                                                                                                   |
| --------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| Trust the certificate in Windows        | Other applications can use that trust, and Mannie cannot manage its scope.                                          |
| Trust every certificate on a hostname   | A different certificate at the same hostname could receive credentials or return page content to the agent.         |
| Disable certificate checking in Browser | Every destination would lose HTTPS server verification, including a redirect or background request to another host. |

## Validation

Extend [V13](../roadmap/milestones.md#4-validations) on Windows with a loopback HTTPS fixture that presents a self-signed certificate. Verify that the server receives no HTTP request before the owner adds trust and that the page and its resources load after a retry. Verify that temporary trust ends with the session, saved trust works in new sessions and conversations and after restarting Mannie, and forgetting closes an affected live connection and blocks a retry. Cover wrong fingerprints, a different hostname using the same certificate, certificate rotation, expiry during an active session, an expired certificate in a fresh session and a certificate with the wrong hostname. Verify that a port sharing the same certificate still needs its own site approval, that an unapproved redirect stays blocked, and that an agent or background request cannot create an exception. Exercise ordinary HTTPS navigation alongside these cases.
