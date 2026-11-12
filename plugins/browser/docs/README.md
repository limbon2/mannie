# Browser

|              |                                                                                                                                                                                      |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Building                                                                                                                                                                             |
| Plugin       | `browser`                                                                                                                                                                            |
| Last updated | 2026-11-13                                                                                                                                                                           |
| Related      | [ADR-0064](../../../docs/adr/0064-give-agents-an-isolated-browser-beside-the-conversation.md), [ADR-0065](../../../docs/adr/0065-let-browser-access-follow-the-conversation.md), D58 |

## Intent

The owner wants agents to navigate live pages, press buttons and complete forms in a browser that the owner watches and uses beside the conversation. The pane shows the real page, which the agent drives through the same tab. Agents can sign in with credentials the owner supplies for that session.

## Behavior

- The Browser plugin starts off. It works on Windows, where the Microsoft Edge WebView2 runtime that Mannie already needs renders the pages. On other systems the plugin does not start, and the Plugins page says that it needs Windows.
- A conversation has its own browser session, cookies and storage. Browser tabs open in the shell's side-panel tab strip. Each panel shows its tab's live page at the panel's size. The owner uses Take over and Resume agent to switch control, and an agent can ask for control while the owner has it. Ending the session or quitting Mannie ends its processes and clears its data.
- Under agent control the page takes no input from the owner. In takeover the owner clicks, scrolls, selects and types in the page itself, and its context menu and browser keys work without save, print, downloads and DevTools. "Focus page" moves the keyboard into the page, and F6 or Tab past the page's ends moves it back to Mannie.
- The page hides while one of the app's dialogs, menus, selects or popovers is open, and the pane shows a still of it there. A tooltip over the page is cut out of it.
- The live tab shows an agent cursor at the points where the agent clicks, drags and scrolls. A press briefly highlights the point. The cursor disappears after a short pause and when the owner takes over or the pane hides.
- A line under the page says what the agent last did: a click, a fill, a choice, a key or a scroll.
- The owner can allow an exact origin for the conversation and revoke it in the conversation permissions. The grant covers observations and interactions at that origin, including localhost and intranet sites. A request to another origin fails at once, and after the owner allows it the browser sends it again. A site can load its ordinary resources and make background requests.
- To let the agent complete a sign-in form, the owner supplies a password or code in the pane's credentials dialog. The browser keeps the value in memory for that session and binds it to the supplied origin. The agent receives a reference it can use to fill a field.
- The agent can read an outline and page text, request a screenshot, follow links, scroll, press keys, drag to sort lists and move items, and interact with forms. Drag and drop works in the page for the owner as in any browser. The outline includes checkbox states when available. The agent can batch element clicks and observe once after the batch. A stale or ambiguous target stops the batch. After the owner used a tab in takeover, the agent gets no screenshot of that document until the tab navigates.
- A point click or drag uses the image pixels and revision of a screenshot returned by the latest observation. The screenshot reports its JPEG dimensions. A later text-only observation supersedes the screenshot. If a point target is stale, the agent requests a fresh screenshot and chooses the point again.

## Fit

The plugin registers browser tools and a conversation side panel. The desktop's browser service hosts each session in a WebView2 environment of its own, checks site approvals, and places the native pages over the pane. The plugin uses generated IPC, the SDK's floating-layer tracker and existing tool image results. [ADR-0064](../../../docs/adr/0064-give-agents-an-isolated-browser-beside-the-conversation.md) and [ADR-0065](../../../docs/adr/0065-let-browser-access-follow-the-conversation.md) define the policy and trust boundary.

## Design

The pane follows the conversation's status through the channel `browser_follow`; that subscription is the lease of the session's native pages, so they hide when no pane follows the conversation and when the app page loads again. The viewport is a placeholder that `utils/browser-surface.ts` measures and reports with `browser_place`, with the rules of `utils/browser-geometry.ts` for overlays. While an overlay hides the page, `browser_still` gives the pane a still image, which never reaches a model. `browser_focus` moves the keyboard into a page, and the status says when focus left a page, so the pane can put it on its own controls.

The agent cursor is drawn by Mannie in the tab's native window. It follows the viewport coordinates of the agent's pointer input at the page's display scale. Page content and browser screenshots contain only the page.

Agent tools use the same session through the host's browser service. The service passes each agent operation through the runtime's approval flow. Screenshots use the existing tool image result. A session's folder and protected form values live only for the session. The core keeps conversation site grants with the conversation's permissions.

## Acceptance

[V13](../../../docs/roadmap/milestones.md#4-validations) checks the browser and its approval boundary on Windows: live form and sign-in flows, navigation redirects, popups, stale targets, owner takeover, teardown and credential redaction.

- The driver's tests use a scripted host on every system: `cargo test --locked -p mannie-tools --lib browser`.
- The real-engine tests use WebView2 and loopback sites: `cargo test --locked -p mannie-desktop-core --lib real_webview2 -- --ignored --test-threads=1`.
- The desktop test `plugins/browser/tests/e2e/browser.test.ts` checks that the page follows its placeholder, hides under dialogs, takes the keyboard with "Focus page" and returns it with F6, refuses owner input under agent control, comes back after the app reloads, shows the agent cursor at two click targets, clears it on takeover, and runs the agent flows.
- By hand: browse, type, scroll, select and play a video; drag the splitter; use the context menu and browser keys; take over and resume; let an agent work with the pane closed, the window minimized and in the tray; try a download, a drop from Explorer and a print; quit while an approval is pending, then confirm that no WebView2 process of the session and no `session-*` folder remain.

A page's own `window.print()` still opens the print dialog in takeover and under agent control: WebView2 has no setting that refuses it, and refusing it from the page would need a script that Mannie injects, which ADR-0064 rules out. The print key and the context menu's Print item are off.
