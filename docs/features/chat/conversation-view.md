# Conversation view

|              |                                                                                                                                                                                                                                                                                                                                                                 |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                                        |
| Milestone    | M1                                                                                                                                                                                                                                                                                                                                                              |
| Last updated | 2026-10-16                                                                                                                                                                                                                                                                                                                                                      |
| Part of      | [Chat](README.md)                                                                                                                                                                                                                                                                                                                                               |
| Related      | [ADR-0020](../../adr/0020-complete-the-ui-shell-slots.md), [ADR-0021](../../adr/0021-render-model-output-as-markdown-with-micromark.md), [ADR-0004](../../adr/0004-keep-system-access-in-the-rust-core.md), [ADR-0019](../../adr/0019-keep-visible-text-in-message-catalogs.md), [Desktop app](../../architecture/desktop-app.md#5-rendering-untrusted-content) |

## Design

### Layout

- The conversation list is a sidebar panel ([ADR-0020](../../adr/0020-complete-the-ui-shell-slots.md)) with two groups: the projects with their conversations ([projects](projects.md#the-sidebar)), and the general conversations with a "New conversation" action, most recent first. A conversation with a running reply shows it. Each entry has a menu to rename or delete it; deleting asks for confirmation.
- The conversation view is the app's first view, so Mannie opens into it. It opens the conversation the owner had open last, or the most recent one. Its address names the conversation: `?view=chat&conversation=<id>`.
- The view's header shows the conversation's title, the provider profile it uses, with a menu to choose another that is wide enough for its title and the profiles' names, the controls that other features add to the toolbar through the shell ([ADR-0026](../../adr/0026-add-conversation-controls-back-for-child-agents.md)), such as the child agents' button, and the context meter. The meter shows the share of the context budget in use, with the exact counts, whether they are estimated, a link to the latest checkpoint and "Compact now".
- The transcript scrolls by itself, and only the transcript: nothing in a reply makes the pane around it scroll. The message box stays at the bottom. While a reply streams or works with tools, the transcript follows it as long as the owner is at the bottom, also when the window or the message box changes size. Only the owner scrolling up, with the wheel, a touch, a key or the pointer, ends the following. Content that grows never ends it, however much it grows at once, and neither does the transcript moving up because the message box shrank or the window grew. A button brings the owner back to the newest message, and while a tool call waits for approval it reads "Jump to the approval request" and brings the owner to the part that waits, which can be the card of a child agent's task above later parts of the reply. Earlier turns load when the owner scrolls to the top.

### Parts

The conversation view renders each part through the part renderer registered for its kind, and unknown kinds through the shell's generic renderer.

| Part              | Rendering                                                                                                     |
| ----------------- | ------------------------------------------------------------------------------------------------------------- |
| The owner's text  | Plain text that keeps the owner's line breaks                                                                 |
| Reply text        | Markdown                                                                                                      |
| Reasoning         | Collapsed above the reply, with a button that shows it. Markdown once opened.                                 |
| Error             | The message for the failure code, with what the owner can do, and Retry when the reply can be retried         |
| Checkpoint marker | A line between turns that says older turns are summarized for the model, with a button that opens the summary |

- Below the latest reply, a status line says what the run is doing: waiting for a free slot, preparing, compacting, writing, waiting for its child agents with their number, retrying with the cause and a countdown, stopped, interrupted, failed, or cut at the output limit. Durations and counts are formatted with `Intl`. Below the status of a stopped or interrupted reply, Retry is a button that shows its icon, with "Retry" as its label and tooltip.
- When a reply finishes, stops or fails, a status region announces it to screen readers. The streamed text itself is not announced word by word.

### Read-only conversations

A child agent's conversation belongs to the agent that gives it tasks ([child agents](../child-agents/README.md)). When the view opens one, it shows it read-only: without the profile menu, the conversation's menu, the message box and Retry. A footer says that the conversation belongs to a child agent, with "Stop" while the child works.

Chat registers the side panel kind `chat.conversation` ([ADR-0025](../../adr/0025-add-side-panels-to-the-shell.md)), which shows any conversation read-only beside the main view in the same way, with the conversation's title, or a child's number and title, as its tab's label. A conversation that no longer exists says so, with "Close".

### Markdown rendering

The renderer follows [ADR-0021](../../adr/0021-render-model-output-as-markdown-with-micromark.md):

- `micromark` with the GFM extensions parses the text into a syntax tree, which Mannie's own Svelte components render. No component inserts HTML, so nothing in the text becomes markup the renderer did not choose.
- Raw HTML in the text is shown as text.
- Links render as buttons, never as elements with an `href`, so the webview never follows them or connects to their host. Activating one asks the core to check the URL; the core returns it in a normalized form, with internationalized host names shown in their ASCII form, and refuses everything except `http` and `https`. A dialog shows the URL in full with "Open in browser" and "Cancel". "Open in browser" asks the core again, which checks the URL again and opens it with the system's browser.
- Images are never loaded. Their place shows the image's description and a link button to its address.
- Code blocks render as text, with the language from the fence as a label and a "Copy" button that uses the clipboard API and says so when copying fails.
- Tables, task lists, strikethrough and autolinks from GFM render as the elements they describe. Footnotes render as plain text.
- Text over 200 KB, or nesting deeper than 32 levels, renders as plain text.
- Short texts are parsed on the main thread. Longer or denser texts, whose parse could take long, are parsed in a Web Worker that gives up after three seconds and shows the text as plain text instead ([ADR-0021](../../adr/0021-render-model-output-as-markdown-with-micromark.md), decision 8).
- While a reply streams, the text is parsed at most once per frame, and the part of the text before the last complete block is parsed only once.

### The message box

- Enter sends, Shift+Enter starts a new line, and Escape stops the reply while the box has the focus. The send button shows its icon, with "Send" as its label and tooltip, and becomes "Stop" while a reply runs.
- The box keeps its text when sending fails, and says why. When no provider profile exists, the view says so and offers to open the provider settings.
- The first message of a new conversation opens the conversation in a pane of its own, in place of the new conversation's pane. Once the conversation has loaded, the new pane's message box takes the focus, unless the owner moved the focus elsewhere meanwhile. Opening a conversation from the list or reloading the window does not put the focus there.

### Settings sections

Each section is a page of the settings ([ADR-0020](../../adr/0020-complete-the-ui-shell-slots.md)).

- Providers: the profiles with their model and interface, which one is the default, and for each one Edit, Test, Make default and Delete. Adding and editing happen in a dialog with the profile's fields, which show example values while they are empty, and the help of the interface, base URL, API key, reasoning effort and tools behind a "(?)" button beside each label. The API key field never shows a stored key: its placeholder says that a key is saved, a key typed into it replaces the saved one, and "Remove the key" removes it. Before saving does either, the dialog says so. The test's results list each check with its outcome.
- Instructions: the owner's instructions in a text area, with how many of their 32,768 characters they use, and "Save".

### Commands

The command palette offers "New conversation", the commands of [projects](projects.md#commands), and, while a conversation is open, "Rename the conversation" and "Compact the conversation".

### Message keys

The view's text uses keys under `chat_`, and the Markdown renderer's under `markdown_`. The text of conversations (the owner's messages, replies, reasoning and summaries) is content and comes from no catalog. Elements that show content carry `data-content`, so the pseudo-locale check of the end-to-end tests skips them.

### Loading, empty and error states

| View              | Loading          | Empty                                                   | Error                                                                     |
| ----------------- | ---------------- | ------------------------------------------------------- | ------------------------------------------------------------------------- |
| Conversation list | Skeleton entries | "No conversations yet"                                  | What failed and "Try again"                                               |
| Conversation      | Skeleton turns   | The message box with a hint about what the owner can do | What failed and "Try again"; a conversation that no longer exists says so |
| Provider settings | Skeleton rows    | "No provider profiles yet" and "Add a profile"          | What failed, and the cause from the core for a refused change             |

### Threat notes

- T-3: the renderer produces only the elements of the syntax tree it knows, never raw HTML, never an element with an `href` or `src` that points outside the app, never frames, forms, styles or meta tags. The hostile corpus of V1, extended with hostile Markdown, runs through the renderer in unit tests and through the real provider path in the end-to-end tests, and must run no script and open no connection.
- Opening a link always goes through the core, which checks the URL twice. A compromised webview could open an `http` or `https` URL without asking the owner, which is within the residual risk of [security](../../architecture/security.md#8-if-the-webview-is-compromised).
- Copying uses the clipboard API on the owner's click; the webview never reads the clipboard.

### Platform differences

The clipboard API needs a secure context, which the app's origin is on both webviews. WebKitGTK can refuse a write that does not follow a click; the button then says that copying failed.

## Acceptance

- The owner creates, opens, renames and deletes conversations, sends messages, stops and retries replies, opens reasoning, switches profiles and compacts, all from the keyboard (end-to-end tests).
- Axe finds no WCAG 2.1 AA violation in the conversation view, the conversation list, the dialogs and the settings sections, in the light and dark themes.
- In the pseudo-locale, every piece of text outside content comes from the catalog, and no layout overflows.
- The hostile corpus runs no script and opens no connection, through the renderer's unit tests and through the end-to-end path from the mock provider.
- A link opens only after the dialog shows its normalized URL and the owner confirms, and `javascript:`, `data:` and `file:` links do not open.
- A reply of 32 KB streams at 200 tokens per second while the view keeps up and answers input within 50 ms (performance test).
