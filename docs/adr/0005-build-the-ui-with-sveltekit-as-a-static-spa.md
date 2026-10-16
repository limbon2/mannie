# ADR-0005: Build the UI with Svelte 5 and SvelteKit as a static single-page app

|          |                                                                                                                                                               |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted                                                                                                                                                      |
| Date     | 2026-10-13                                                                                                                                                    |
| Deciders | Owner                                                                                                                                                         |
| Related  | [ADR-0004](0004-keep-system-access-in-the-rust-core.md), [ADR-0006](0006-use-shadcn-svelte-through-its-cli.md), [Desktop app](../architecture/desktop-app.md) |

## Context

Verified 2026-10-13:

- The owner chose Svelte ([decision D3](../roadmap/milestones.md#5-decisions)).
- Svelte 5.57.2 is current. SvelteKit 3.0.1 is the current major version. It requires Vite 8, TypeScript 6 and Node 22 or newer, and it replaces `$app/stores` with `$app/state`. `@sveltejs/adapter-static` 4.0.0 belongs to it.
- Tauri's SvelteKit guide uses adapter-static with an `index.html` fallback, turns off server-side rendering in the root layout, and avoids prerendering so that load functions can call Tauri APIs.
- shadcn-svelte's SvelteKit setup uses the `$lib` alias.
- TypeScript 7.0.2 has been released, but SvelteKit 3 declares a peer range of `^6.0.0`.

## Decision

1. The UI is a SvelteKit 3 app built with adapter-static (fallback `index.html`), with `ssr = false` and `prerender = false` in the root layout. There are no server routes, form actions or server hooks.
2. Runes mode is forced for every project file through `compilerOptions` in `svelte.config.js`. Dependencies in `node_modules` are not affected.
3. TypeScript runs in strict mode on the 6.x line until the Svelte toolchain supports TypeScript 7.
4. Application state lives in `.svelte.ts` modules that use runes. New code does not use the legacy store API, and `$app/state` is used instead of `$app/stores`.
5. Routes exist for top-level views only. Inside a view, features add UI through the shell's slots ([extension points](../architecture/extension-points.md)).

## Consequences

- Positive: SvelteKit's conventions, the `$lib` alias that shadcn-svelte expects, and the `sv` and `svelte-check` tools.
- Negative: most of SvelteKit's server features go unused, and SvelteKit 3 is new (risk R3).

## Alternatives considered

| Option                                    | Why not chosen                                                                                        |
| ----------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| Svelte with plain Vite and a small router | shadcn-svelte supports it, but it has none of SvelteKit's conventions; it is the fallback if V1 fails |
| Prerendered SvelteKit                     | Load functions would run at build time, where Tauri APIs do not exist                                 |

## Validation

V1.
