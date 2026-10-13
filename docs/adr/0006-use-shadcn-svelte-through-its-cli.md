# ADR-0006: Add and update UI components only through the shadcn-svelte CLI and its documentation

|          |                                                                                                                                              |
| -------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D3)                                                                                                                                |
| Date     | 2026-10-13                                                                                                                                   |
| Deciders | Owner                                                                                                                                        |
| Related  | [ADR-0005](0005-build-the-ui-with-sveltekit-as-a-static-spa.md), [Desktop app](../architecture/desktop-app.md), [AGENTS.md](../../AGENTS.md) |

## Context

Verified 2026-10-13:

- The owner chose shadcn-svelte ([decision D3](../roadmap/milestones.md#5-decisions)) and asked for this rule. Coding agents tend to rebuild shadcn components from memory, often from the React version, instead of using the CLI and reading the documentation. The result drifts from upstream, gets accessibility details wrong and calls APIs that do not exist.
- shadcn-svelte copies component source into the project. Its CLI, version 1.7.0, has `init`, `add` (with `--overwrite`, `--yes` and `--all`), `apply` for presets, and `registry build`. It has no update or diff command, and the base color cannot be changed after `init`.
- The documentation is published as Markdown: an index at `https://shadcn-svelte.com/llms.txt` and one page per component at `https://shadcn-svelte.com/docs/components/<name>.md`.
- An official agent skill installs with `pnpm dlx skills add huntabyte/shadcn-svelte`. It reads `components.json` and gives agents guidance that matches the project.
- The components are built on bits-ui 2.19.5 and Tailwind CSS 4.3.3.

## Decision

1. shadcn-svelte is the component system of the desktop UI. bits-ui, the primitive layer under it, is used directly only inside Mannie's own components, and only where shadcn-svelte has no component.
2. The CLI is a devDependency pinned to an exact version. Components are added only with it (`pnpm shadcn-svelte add <name>`). Nobody writes them by hand, pastes them from other projects or ports them from the React version.
3. Before using or adding a component, the author reads its documentation page and checks `components.json` and `src/lib/components/ui/` for what is already installed.
4. `src/lib/components/ui/` contains CLI output only, formatted by the repository formatter. Customization goes through theme tokens (CSS variables) and app components in `src/lib/components/app/`. A change that has to be made inside `ui/` is recorded with its reason in the patch register, `src/lib/components/ui/PATCHES.md`, so that it can be applied again after an update.
5. An update bumps the pinned CLI in a dedicated change, re-adds the components with `--overwrite`, reviews the diff, applies the registered patches again and runs the UI checks.
6. The official shadcn-svelte agent skill is installed in the repository for Claude Code and Codex in M0, and agents use it.
7. Icons come from `@lucide/svelte`.
8. When shadcn-svelte has no matching component, the author composes one in `src/lib/components/app/` from installed components and bits-ui primitives.

## Consequences

- Positive: components stay close to upstream, accessible and upgradable, and agents follow one documented path.
- Negative: without an update command, updates are manual re-adds, and the patch register needs discipline.

## Alternatives considered

| Option                           | Why not chosen                              |
| -------------------------------- | ------------------------------------------- |
| Hand-written components          | Produces the drift described in the context |
| Another Svelte component library | The owner chose shadcn-svelte               |

## Validation

None needed: the owner decided this (D3). V1 confirms that the CLI works with SvelteKit 3 and Tailwind CSS 4.
