# Browser changelog

Changes to the Browser plugin. Releases collect its entries in the [Mannie changelog](../../CHANGELOG.md).

## Unreleased

### Added

- Show an agent cursor over the live page for clicks, drags and scrolls.

- Add a conversation browser on Windows with site approvals, owner takeover and session credentials. The pane shows the live page in a WebView2 webview that the agent drives through the same tab ([ADR-0063](../../docs/adr/0063-give-agents-an-isolated-browser-beside-the-conversation.md)).
- Hide the page under the app's dialogs and menus and show a still of it there; cut tooltips out of it.
- Move keyboard focus into the page with "Focus page", and back to Mannie with F6 or Tab.
- Fail a request to an unapproved site at once and send it again after the owner approves it.
- Withhold screenshots of a document that the owner used in takeover until its tab navigates ([ADR-0064](../../docs/adr/0064-let-browser-access-follow-the-conversation.md)).
- Let agents request control and click a batch of page elements. Show Browser tabs in the shared side-panel strip.
- Reuse site approvals across turns in the same conversation and show agents when a viewport screenshot is available.
- Say on the Plugins page that Browser needs Windows, on other systems.
- Let an agent press a key, such as Enter after a fill, without observing the page again, and say which keys it can press.
- Let agents drag from one element or point to another, with HTML drag and drop and pointer-based sorting, and give list items, rows and cells references named by their text.
- Keep drag and drop working in the page for the owner, which turning off drops from other programs had stopped.

### Fixed

- Fix coordinate clicks and drags on pages that repaint continuously, including animated canvases.
- Report screenshot dimensions from the JPEG and tell agents how to refresh stale point targets.
