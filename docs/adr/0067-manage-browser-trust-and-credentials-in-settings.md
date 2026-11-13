# ADR-0067: Manage Browser trust and credentials in Settings

|          |                                                                                                                                                                                                                                                                         |
| -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D62)                                                                                                                                                                                                                                                          |
| Date     | 2026-11-14                                                                                                                                                                                                                                                              |
| Deciders | Owner                                                                                                                                                                                                                                                                   |
| Related  | [ADR-0025](0025-add-side-panels-to-the-shell.md), [ADR-0064](0064-give-agents-an-isolated-browser-beside-the-conversation.md), [ADR-0065](0065-let-browser-access-follow-the-conversation.md), [ADR-0066](0066-let-the-owner-trust-self-signed-browser-certificates.md) |

## Context

The owner wants the Browser toolbar to work like Terminal and wants to manage certificates and credentials in Settings. The owner chose saved certificate trust for all Browser sessions and reusable site credentials with separate consent in each conversation.

## Decision

1. Register a Browser settings page with trusted hosts and site credentials. **Settings > Browser** holds both sections. The conversation's Browser menu lists live tabs and can create a tab under owner control. The tab strip closes the pane and the live tab can be opened again. End session closes all tabs in the session. The Browser pane has no separate native tab close button. This refines [ADR-0065](0065-let-browser-access-follow-the-conversation.md), decision 5.
2. A side-panel kind may set `hideHeading` to hide its heading visually. The heading still takes focus when the panel opens, and the panel shows a visible focus indicator. Browser uses this option. During takeover, clicking the page focuses it and F6 from the pane enters it. F6 or Tab past either end returns focus to Mannie. This refines [ADR-0025](0025-add-side-panels-to-the-shell.md), decisions 2 and 5, and [ADR-0064](0064-give-agents-an-isolated-browser-beside-the-conversation.md), decision 6.
3. All certificate trust is saved in Settings for the verified hostname and fingerprint across HTTPS ports. Forgetting the entry ends sessions that allowed that pair. Historical session confirmations remain in the audit record. Only entries expressly saved by the owner appear in trusted hosts. The certificate validation, expiry cutoff, journal and independent origin consent of [ADR-0066](0066-let-the-owner-trust-self-signed-browser-certificates.md) still apply. This replaces its session trust and pane controls.
4. The owner saves a password or code against a canonical HTTP or HTTPS origin and a label. The master stores the value in the OS credential store and SQLite stores the label, origin, opaque ID and change journal. A saved credential applies across Browser sessions and restarts. It stays available until the owner removes it. Removal revokes its conversation grants and disables use before purging the store entry. An interrupted write is recoverable, and replacement gets a fresh ID. The old session command refuses changes and directs the owner to Browser Settings. This supersedes the session scope and location in [ADR-0064](0064-give-agents-an-isolated-browser-beside-the-conversation.md), decision 10.
5. A site grant exposes the credential's label and opaque reference in an observation of that exact origin. Filling a field with the value requires an additional grant for that credential in the conversation that uses it. The owner can revoke the grant in Conversation Permissions. The grant does not transit across conversations. Full access does not grant it. The service checks the live target, site, grant and current credential before dispatch through the browser's protected fill. The value stays out of tool arguments, journals, logs, transcripts and exports.

## Consequences

Owners can manage Browser trust and site credentials before starting a session. A removed credential stops authorizing fills. Sign-ins already established on the website can remain active until the session ends.

## Validation

[V13](../roadmap/milestones.md#4-validations) covers the Settings workflow, independent credential and origin approvals, persistence and revocation, redaction, and the WebView2 certificate and focus checks.
