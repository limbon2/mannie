# ADR-0079: Highlight code and shell commands with Shiki mapped to CSS classes

|          |                                                                                                                                                                                                                                                                                                   |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D73)                                                                                                                                                                                                                                                                                    |
| Date     | 2026-11-24                                                                                                                                                                                                                                                                                        |
| Deciders | Owner                                                                                                                                                                                                                                                                                             |
| Related  | [ADR-0004](0004-keep-system-access-in-the-rust-core.md), [ADR-0006](0006-use-shadcn-svelte-through-its-cli.md), [ADR-0017](0017-license-and-dependency-policy.md), [ADR-0021](0021-render-model-output-as-markdown-with-micromark.md), [Tool activity](../features/system-tools/tool-activity.md) |

## Context

- [ADR-0021](0021-render-model-output-as-markdown-with-micromark.md), decision 10, left code blocks without syntax highlighting in M1 and reserved choosing a syntax highlighter for a decision of its own.
- On 2026-11-24 the owner asked for syntax highlighting on shell commands in tool details and chose Shiki ([decision D73](../roadmap/milestones.md#5-decisions)), which also covers SQL statements, JSON arguments and Markdown fenced code blocks through the `CodeBlock` component ([ADR-0006](0006-use-shadcn-svelte-through-its-cli.md)).
- Mannie's Content Security Policy (`style-src: 'self'`, [ADR-0004](0004-keep-system-access-in-the-rust-core.md)) and Markdown renderer tests ([ADR-0021](0021-render-model-output-as-markdown-with-micromark.md), decision 3) forbid inline `style` attributes and `{@html}`. `shadcn-svelte`'s default Shiki adapter emits inline `style="--shiki-light:...;--shiki-dark:..."` on every token, which the CSP blocks.
- Verified 2026-11-24:
  - `shiki` 4.6.0 (MIT; all transitive dependencies admitted by `scripts/check-licenses.mjs`, [ADR-0017](0017-license-and-dependency-policy.md)) provides `shiki/core`, a pure-JavaScript regular-expression engine (`shiki/engine/javascript`, requiring no WebAssembly) and per-language ES modules under `shiki/langs/*.mjs` (including `powershell`, `shellscript`, `bat`, `fish`, `json`, `sql`, `rust`, `typescript`, `javascript`, `svelte`, `python`, `yaml`, `toml`, `html`, `css`, `markdown` and `diff`).
  - `CodeBlock.Line` renders `{ content, class }` token objects as plain text `<span>` elements without `{@html}` and omits the `style` attribute when `token.style` is `undefined`.

## Decision

1. Add `shiki` 4.6.0 (exact pin) to `apps/desktop/package.json`. This supersedes [ADR-0021](0021-render-model-output-as-markdown-with-micromark.md), decision 10, in part.
2. Run Shiki through `createHighlighterCore` with `createJavaScriptRegexEngine({ forgiving: true })` in `apps/desktop/src/lib/utils/highlighter.ts`. Dynamically import `shiki/core`, `shiki/engine/javascript` and each language grammar on demand so neither the engine nor any grammar enters the app's first bundle.
3. Map TextMate scopes to semantic CSS classes instead of inline styles:
   - Register a single in-memory `ThemeRegistration` (`mannie`) that assigns fixed sentinel colors to semantic scope groups (comments, strings, keywords, functions and commands, variables, numbers and constants, types, operators, diff deletions and diff insertions).
   - Map each token's sentinel color to a `.syntax-*` CSS class whose colors live in `apps/desktop/src/app.css` for both light and dark themes at WCAG 2.1 AA contrast. Never emit a `style` property on any token.
4. Keep untrusted content and responsiveness bounded:
   - Render plain text immediately during server-side rendering, while a grammar loads, for unknown languages, and whenever a block exceeds 32 KiB or contains a line longer than 2,000 characters.
   - Tokenize streaming code incrementally line by line using Shiki's `GrammarState` in `apps/desktop/src/lib/state/highlighted-code.svelte.ts`.
   - Render every token through Svelte text bindings inside `CodeBlock.Line`, never `{@html}`.
5. Apply syntax highlighting to:
   - shell commands in tool details, approvals and background command panels, selecting `powershell`, `bat`, `fish` or `bash` from the command's shell path (defaulting remote SSH commands and installed program runs to `bash`);
   - database SQL statements (`sql`) and external-tool or raw JSON arguments (`json`);
   - Markdown fenced code blocks (`MarkdownCode.svelte`) when the fence names a supported language or alias.

Earlier decisions that this ADR changes:

| Earlier decision                                                                | Change                                                                                                                                |
| ------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| [ADR-0021](0021-render-model-output-as-markdown-with-micromark.md), decision 10 | Superseded in part: code blocks and tool command/argument blocks highlight with `shiki` 4.6.0 mapped to CSS classes, with no `style`. |

## Consequences

- Positive: shell commands, SQL statements, JSON arguments and Markdown code blocks are easier to scan, obey the strict `style-src: 'self'` Content Security Policy, follow light and dark themes automatically and stay out of the initial bundle.
- Negative: adds `shiki` 4.6.0 to the desktop app's frontend dependencies and adds lazy-loaded grammar chunks to the build output. Highlighting arrives asynchronously on the first use of each language.
- Risks: a hostile or huge code block could stall the JavaScript regex engine; the 32 KiB total cap, 2,000-character per-line cap and `forgiving: true` engine option limit tokenization cost and fall back to plain text.

## Alternatives considered

| Option                                                   | Why not chosen                                                                                             |
| -------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| Default `shadcn-svelte` Shiki adapter with inline styles | Emits `style` attributes on every token, which `style-src: 'self'` blocks and `Markdown.test.ts` forbids.  |
| Custom hand-written shell tokenizer                      | Avoids a dependency but only covers shells and does not cover SQL, JSON or Markdown fenced code blocks.    |
| Oniguruma WASM engine (`shiki/engine/oniguruma`)         | Requires shipping and instantiating WebAssembly under the webview CSP when the JavaScript engine suffices. |

## Validation

Unit tests in `tests/apps/desktop/src/lib/utils/highlighter.test.ts` verify PowerShell, Bash, cmd (`bat`), SQL, JSON and Rust tokenization, shell path detection, zero `style` properties on all emitted tokens, unknown-language fallback and size-cap fallback. `Markdown.test.ts`, `svelte-check`, ESLint, `check-css`, `check-first-bundle` and `check-licenses` confirm CSP compliance, bundle isolation and license compliance.
