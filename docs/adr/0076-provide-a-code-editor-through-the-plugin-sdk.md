# ADR-0076: Provide a code editor through the plugin SDK

|          |                                                                                                                                                                                                                                                                                                                                                                                                                         |
| -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted                                                                                                                                                                                                                                                                                                                                                                                                                |
| Date     | 2026-11-22                                                                                                                                                                                                                                                                                                                                                                                                              |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Related  | [ADR-0004](0004-keep-system-access-in-the-rust-core.md), [ADR-0017](0017-license-and-dependency-policy.md), [ADR-0034](0034-build-plugins-on-a-plugin-sdk.md), [ADR-0035](0035-find-plugins-at-build-time-in-folders-of-their-own.md), [ADR-0053](0053-let-plugins-offer-terminals.md), [ADR-0075](0075-let-plugins-send-network-requests-from-collections.md), [Technology stack](../architecture/technology-stack.md) |

## Context

- The owner asked for a real code editor with syntax highlighting (and variable completion where applicable) for editing and viewing structured payloads such as JSON, YAML, XML, HTML and plain text in plugins, beginning with the Network Client plugin's request body, response body and OpenAPI paste views.
- A plugin's UI cannot add npm packages of its own ([ADR-0035](0035-find-plugins-at-build-time-in-folders-of-their-own.md), decision 7) and imports only `@mannie/plugin-sdk`, Svelte and its own files ([ADR-0034](0034-build-plugins-on-a-plugin-sdk.md), decision 11).
- The desktop app enforces a strict Content Security Policy with `style-src 'self'` and no `'unsafe-inline'` ([ADR-0004](0004-keep-system-access-in-the-rust-core.md)). Any editor library that inserts `<style>` elements into `document.head` triggers CSP violations.
- Adding a code editor must not increase the desktop app's startup bundle size, matching the lazy-loading requirement established for xterm.js ([ADR-0053](0053-let-plugins-offer-terminals.md), decision 7).
- Facts verified on 2026-11-22:
  - CodeMirror 6 (`@codemirror/state` 6.7.6, `@codemirror/view` 6.43.15, `@codemirror/language` 6.13.1, `@codemirror/commands` 6.11.1, `@codemirror/autocomplete` 6.20.4, `@codemirror/lang-json` 6.0.2, `@codemirror/lang-yaml` 6.1.3, `@codemirror/lang-xml` 6.1.0, `@codemirror/lang-html` 6.4.12 and `@lezer/highlight` 1.2.5) is modular, MIT-licensed ([ADR-0017](0017-license-and-dependency-policy.md)), assigns inline styles through `element.style.cssText` rather than `setAttribute("style", ...)`, and needs no Web Workers or external asset files.
  - CodeMirror's `style-mod` dependency inserts a `<style>` element when mounting rules onto `document`, even though it already supports constructible `CSSStyleSheet` objects via `adoptedStyleSheets`.

## Decision

1. Expose a shared code editor through the TypeScript plugin SDK:
   - `@mannie/plugin-sdk/code-editor` exports `createCodeEditor`, `CodeEditorLanguage`, `CodeEditorHighlightMode`, `CodeCompletionItem`, `CodeEditorOptions` and `CodeEditorHandle`.
   - `@mannie/plugin-sdk/components` exports `CodeEditor`, a Svelte 5 component backed by `createCodeEditor` that supports multiline editing, compact single-line mode and inline URL/variable token highlighting, and renders a semantic fallback while the async chunk loads or in Node tests.
2. Load all `@codemirror/*` and `@lezer/*` packages exclusively through dynamic `import()` inside `createCodeEditor`, and load each language grammar only when requested. `scripts/check-first-bundle.mjs` checks that CodeMirror never enters the desktop app's first bundle.
3. Alias `style-mod` in `apps/desktop/vite.config.ts` to `apps/desktop/src/lib/utils/style-mod-csp.ts`, which preserves the `StyleModule` API and mounts CSS rules through constructible `CSSStyleSheet` objects in `adoptedStyleSheets` on both `Document` and `ShadowRoot`, matching Mannie's xterm.js stylesheet approach and emitting zero `<style>` tags under `style-src 'self'`.
4. Style the editor from Mannie's CSS variables and accessible light and dark syntax palettes, reacting to theme switches through a `MutationObserver` on `document.documentElement`. Leave bare `Tab` free for keyboard focus traversal unless an autocomplete popup is open.

## Alternatives considered

- Monaco Editor: ships multi-megabyte bundles and Web Workers, relies on inline `<style>` injection and AMD/worker wiring, and is much heavier than needed for structured payload editing and inspection.
- Highlighted `<pre>` overlaid on a `<textarea>`: fragile cursor and scroll synchronization, poor folding and editing ergonomics, and breaks down on long wrapped lines.

## Consequences

- Any core view or plugin can use `CodeEditor` through the SDK with JSON, YAML, XML, HTML or plain text highlighting, read-only viewing and optional `{{variable}}` completion.
- Startup bundle size stays unchanged because CodeMirror loads only when a code editor actually mounts.
- Upgrading CodeMirror 6 in the future requires keeping `style-mod-csp.ts` aligned with the `StyleModule` API expected by `@codemirror/view`.
