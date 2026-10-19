# ADR-0004: Keep logic and system access in the Rust core and treat the webview as untrusted

|          |                                                                                                                                                                                                   |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted                                                                                                                                                                                          |
| Date     | 2026-10-13                                                                                                                                                                                        |
| Deciders | Owner                                                                                                                                                                                             |
| Related  | [ADR-0003](0003-build-the-desktop-app-on-tauri-2.md), [ADR-0007](0007-generate-ipc-bindings-from-rust.md), [Desktop app](../architecture/desktop-app.md), [Security](../architecture/security.md) |

Later changes: [ADR-0034](0034-build-plugins-on-a-plugin-sdk.md) refines decision 3: a plugin's command gets its permission at build time through its inlined plugin, and only the main window receives grants.

## Context

Verified 2026-10-13:

- The webview renders content that Mannie does not control: model output, tool output and file contents. A rendering bug, or a bug in a dependency, can let that content run script in the webview.
- By default, Tauri 2 lets every window call every registered app command. `tauri_build::AppManifest::commands` changes this so that each command needs a capability grant.
- Tauri plugins such as fs, shell and http give JavaScript access to files, processes and the network.
- Tauri offers an isolation pattern that checks IPC messages in an isolated frame before they reach the core.
- On Windows, the `useHttpsScheme` window option decides whether the app is served from `https://tauri.localhost` or `http://tauri.localhost`. Changing it later changes the origin, and everything in web storage is lost.

## Decision

1. All domain logic, file and process access, network access, credentials and policy checks live in the Rust core. The webview displays state and sends the owner's intents.
2. The core treats every command from the webview as untrusted input. It validates the arguments and applies the same policy checks as for a model's request, so no command acts on a path, a shell string or a URL without them.
3. Every app command needs a capability grant through `AppManifest::commands`, and only the main window receives grants.
4. Mannie does not use the fs, shell or http plugins, or any other plugin that gives JavaScript direct access to files, processes or the network.
5. `withGlobalTauri` is false. The webview cannot navigate away from the app's origin or open new windows.
6. The Content Security Policy is:
   - `default-src 'self'`;
   - `script-src 'self'`, with no inline scripts and no eval;
   - `style-src 'self'`;
   - `img-src 'self' data: blob:`, so no remote images load;
   - `connect-src` limited to Tauri's IPC endpoints;
   - `object-src`, `frame-src` and `base-uri` set to `'none'`.

   V1 confirms that the component stack works under this policy. Any exception needs a new ADR.

7. Model and tool output is rendered as Markdown without raw HTML. Links never open inside the webview. Clicking one sends a request to the core, which asks the owner before opening it in the system browser.
8. `useHttpsScheme` is true from the first release and does not change.
9. The isolation pattern is not used. Validation and authorization already happen in the core, and an isolation hook cannot tell a malicious approval from a real one, so it would add a frame and complexity without closing a gap. This is reconsidered if third-party script ever has to run in the webview.
10. The webview stores only UI preferences. Transcripts, settings and secrets stay in the core.

## Consequences

- Positive: script that runs in the webview cannot read files, start processes or reach the network directly, and authorization is enforced in one place.
- Negative: every UI need goes through a Rust command. Generated bindings ([ADR-0007](0007-generate-ipc-bindings-from-rust.md)) keep this manageable.
- Risks: script in the webview could still call approval commands for pending operations. The policy above makes running such script hard, and [security](../architecture/security.md) records the remaining risk.

## Alternatives considered

| Option                                     | Why not chosen                                                 |
| ------------------------------------------ | -------------------------------------------------------------- |
| fs, shell and http plugins for convenience | Any script in the webview would get the same reach as the core |
| The isolation pattern                      | Adds a frame for checks that the core already performs         |
| Business logic in TypeScript               | Puts trusted logic in the least trusted component              |

## Validation

V1: the component stack runs under the policy above on WebView2 and WebKitGTK, a command without a capability grant is refused, and model output taken from a corpus of hostile Markdown and HTML runs no script and loads nothing remote.
