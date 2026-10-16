# ADR-0021: Render model output as Markdown with micromark and Mannie's own components

|          |                                                                                                                                                                                                                                                                                              |
| -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted                                                                                                                                                                                                                                                                                     |
| Date     | 2026-10-14                                                                                                                                                                                                                                                                                   |
| Deciders | Owner                                                                                                                                                                                                                                                                                        |
| Related  | [ADR-0004](0004-keep-system-access-in-the-rust-core.md), [ADR-0017](0017-license-and-dependency-policy.md), [Desktop app](../architecture/desktop-app.md#5-rendering-untrusted-content), [Security](../architecture/security.md), [Conversation view](../features/chat/conversation-view.md) |

## Context

Verified 2026-10-14:

- Replies render as Markdown with code blocks. Raw HTML is not rendered, remote images do not load, and links open in the system browser only after the core checks them and the owner confirms ([ADR-0004](0004-keep-system-access-in-the-rust-core.md), decision 7, and [desktop app](../architecture/desktop-app.md#5-rendering-untrusted-content)).
- V1 showed that the webview opens a connection to the host of a frame, a meta refresh or a followed link before the Content Security Policy or the navigation guard refuses it ([M0 validation results](../roadmap/m0-validation.md), finding 3). The renderer must therefore never produce such markup in the first place.
- Model output can be hostile: it may repeat text that a web page or file planted. Text in pathological shapes must not freeze the window either.
- Svelte's `{@html}` inserts a string as markup, and eslint-plugin-svelte reports it as an error.
- markdown-it 15.0.2 is MIT, but it depends on argparse 3.0.2, which is under the PSF-2.0 license that the license policy does not admit ([ADR-0017](0017-license-and-dependency-policy.md)).
- marked 18 is MIT and has no dependencies. Its lexer is built on regular expressions, and its tokens keep character references such as `&amp;` encoded, so rendering them as text needs a decoder of Mannie's own.
- micromark 4.0.3 is a CommonMark parser written as a state machine, without regular expressions on the input. `mdast-util-from-markdown` 2.1.0 turns its output into an mdast syntax tree with decoded text, and `micromark-extension-gfm` 3.0.0 with `mdast-util-gfm` 3.1.0 add tables, task lists, strikethrough, autolinks and footnotes. All are MIT.
- Its parsing time still grows faster than linearly in some shapes, measured on the development machine: 1,000 pairs of `*a` take 12 ms and 75,000 take 103 s, 300 table rows of six columns take 50 ms and 4,000 rows of two take 0.9 s, and 4,000 nested list markers overflow the stack. A realistic reply of 200 KB takes 0.8 s. No option of the parser bounds this.
- micromark decodes named character references through `decode-named-character-reference` 1.3.0, whose browser build assigns `innerHTML` on a detached element. Its default build looks the names up in a table instead.

## Decision

1. The webview parses model output with `mdast-util-from-markdown`, `micromark-extension-gfm` and `mdast-util-gfm`, pinned to exact versions.
2. `apps/desktop/src/lib/utils/markdown.ts` turns the mdast tree into Mannie's own render tree, which has only the node kinds that Mannie renders: paragraphs, headings, emphasis, strong text, strikethrough, inline code, code blocks, block quotes, lists and task lists, tables, thematic breaks, line breaks, text, links and image placeholders. Raw HTML nodes become text. Footnotes, definitions and anything the tree does not know become text.
3. Svelte components in `apps/desktop/src/lib/components/app/markdown/` render the render tree with text bindings. No component uses `{@html}`, and no element receives an `href`, `src`, `srcset`, `style`, form or event attribute from the content.
4. A link renders as a button. Activating it sends the URL to the core, which normalizes it, accepts only `http` and `https`, and returns the normalized URL; a dialog shows it, and the URL opens in the system browser only after the owner confirms, through the core, which checks it again.
5. An image renders as its description and a link button to its address. The webview never loads it.
6. Text over 200 KB, or nesting deeper than 32 levels, renders as plain text.
7. Vite resolves `decode-named-character-reference` to its default build, so the rendering path never assigns `innerHTML`.
8. A text within cheap bounds is parsed on the main thread: at most 32 KiB, 1,000 emphasis and strikethrough delimiters, 1,000 opening brackets, 150 lines with `|`, and 32 nesting levels, which a linear scan checks first. Every other text is parsed in a Web Worker, one text at a time. A parse that takes longer than three seconds is abandoned, the worker is replaced, and the text renders as plain text, so no text can freeze the window or hold up the others.
9. While a reply streams, the renderer parses at most once per animation frame and reuses the render tree of the text before the last complete block, on the main thread or in the worker.
10. Code blocks have no syntax highlighting in M1. A highlighter would be a dependency with its own decision.

## Consequences

- Positive: the only markup in a reply is markup that Mannie's components chose, so script, frames, styles, forms and remote resources cannot appear through it. The window stays responsive whatever the text is: the main thread parses only texts within the cheap bounds, and the worker's time limit catches the rest. The tree keeps positions, which later features can use.
- Negative: Mannie maintains the components for each node kind and the worker. A long or dense reply shows as plain text until the worker's tree arrives, and one that the worker cannot parse in time stays plain text. micromark parses more slowly than regular-expression parsers, which the per-frame parsing and the reuse of finished blocks compensate for.
- Risks: a later GFM version may add node kinds; they render as text until Mannie supports them. A bug in a component could still pass content into an attribute; the hostile corpus tests check every component.

## Alternatives considered

| Option                                         | Why not chosen                                                                                             |
| ---------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| markdown-it with HTML disabled                 | Its argparse dependency is PSF-2.0, which the license policy does not admit                                |
| marked                                         | Regular-expression lexer on hostile input, and encoded character references that need extra code           |
| A sanitizer such as DOMPurify on rendered HTML | Keeps HTML strings and `{@html}` in the path, and relies on the sanitizer's allow list matching the policy |
| Parsing in the Rust core (pulldown-cmark)      | Every delta would cross IPC as a re-rendered tree, and the webview would still need the same components    |
| svelte-exmarkdown                              | Renders a generic element for each HTML tag name in the tree, which leaves the allow list to a dependency  |

## Validation

M1: the hostile HTML corpus of V1, extended with hostile Markdown, runs no script and opens no connection, both in the renderer's unit tests and through the real path from the mock provider in the end-to-end tests. Texts within the cheap bounds parse within 250 ms in the unit tests, and pathological texts go to the worker. A 32 KB reply streams at 200 tokens per second within the performance targets.
