# ADR-0039: Let plugins describe their tool calls in the transcript

|          |                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D37)                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Date     | 2026-10-20                                                                                                                                                                                                                                                                                                                                                                                                                             |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Related  | [ADR-0006](0006-use-shadcn-svelte-through-its-cli.md), [ADR-0013](0013-journal-every-side-effect.md), [ADR-0020](0020-complete-the-ui-shell-slots.md), [ADR-0021](0021-render-model-output-as-markdown-with-micromark.md), [ADR-0034](0034-build-plugins-on-a-plugin-sdk.md), [ADR-0038](0038-let-plugins-run-installed-programs-in-the-owners-projects.md), [Extension points](../architecture/extension-points.md#37-ui-shell-slots) |

Later changes: [ADR-0043](0043-let-plugins-offer-the-tools-of-tool-servers.md) refines decision 1: one tool kind may describe every tool of its plugin that has no entry of its own.

## Context

- The owner wants git tool calls to read well in the transcript: a summary that says what happened, and the diff, the commits or the conflicts that the call returned, laid out for reading ([decision D37](../roadmap/milestones.md#5-decisions)).
- One renderer shows every `tool_call` part (`apps/desktop/src/lib/registrations/tools.ts`). The core's tools get their icons and summaries from switches over the tool's name and the typed outcome in `apps/desktop/src/lib/utils/tools-summaries.ts`, and a plugin's call shows only the tool's name, because its outcome, `plugin`, names nothing but the plugin.
- The slot of part renderers is keyed by the part's kind, the core registers `tool_call`, and a second registration of an ID fails, so a plugin cannot draw its own calls. The journal record, the approvals and the archives belong to that part and stay the core's ([ADR-0013](0013-journal-every-side-effect.md)).
- No diff view exists in the app. A plugin's UI cannot add an npm package or a shadcn-svelte component of its own; the SDK adds and re-exports what plugins use ([ADR-0034](0034-build-plugins-on-a-plugin-sdk.md) d11, [ADR-0006](0006-use-shadcn-svelte-through-its-cli.md)).

## Decision

1. The slot catalog gains `toolKinds`, one entry for each tool of a plugin, keyed by the tool's name: an icon, a summary of a call, an optional component for the call's details, and the words of the tool's failure codes. The `tool_call` renderer uses an entry only when the plugin that registered it is the plugin of the call. The accesses, the approval, the decision and the buttons of the archives stay the core's. Without an entry, such as while the plugin is off, the card shows the generic text that it shows today. The slot has a registration test with a fake entry, as every slot ([ADR-0020](0020-complete-the-ui-shell-slots.md) d5).
2. The journal record of a call, `ToolCallDocument`, and its view name the plugin of a plugin tool's call. The runtime sets it, and a record from before reads as none.
3. A plugin tool's result may carry data for its card: a JSON object of at most 64 KiB and a failure code of 1 to 64 lower-case letters and underscores. A refusal of the arguments may carry a code too. The core stores both in the outcome `plugin`, beside the plugin's ID, and the model never reads them: it reads the output alone. Data that is not an object or is too large is dropped with a log line, as an archive that breaks the rules is. A refusal with a code ends the call completed with that failure, and the card shows a plugin outcome with a failure as failed.
4. The SDK re-exports the types of a tool call's view and the components that plugins' cards and panels use, added through the pinned CLI, such as `tabs`. The theme gains tokens for added and removed lines, with WCAG 2.1 AA contrast in both themes, and a card marks such lines with `+` and `-` as text as well, so no line depends on its color.
5. A card renders its data as text through Svelte's bindings, never as markup ([ADR-0020](0020-complete-the-ui-shell-slots.md) d4). The data comes from the plugin's Rust code, which parsed what it ran, so the UI parses no output of a program.

Earlier decisions that this ADR changes:

| ADR  | Decision | Change                                                                                                                                              |
| ---- | -------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| 0020 | d1       | Refined: the shell gains the slot of tool kinds, through which a plugin describes its tool calls inside the core's `tool_call` renderer             |
| 0034 | d6, d11  | Refined: a plugin tool's result and refusal carry data and codes for its card, and the TypeScript SDK re-exports the types and components cards use |

## Consequences

- Positive: a plugin's calls read like the core's, with an icon, a summary and details, and the transcript stays one kind of part with one journal and one approval flow. A plugin with a richer result needs no change in the core.
- Negative: the stored outcome of a plugin's call grows by up to 64 KiB of data. A card that its plugin no longer registers, because the plugin is off or was removed, falls back to generic text, so its details show only while the plugin is on.
- Risks: data that a later version of a plugin reads differently from the version that stored it. A plugin versions the shape of its data and shows a generic card for a shape that it does not know.

## Alternatives considered

| Option                               | Why not chosen                                                                                                      |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------- |
| Part kinds of plugins                | The journal record, approvals and archives belong to `tool_call`, and a second part kind would split them           |
| Parsing the model's output in the UI | The output is text for the model, in English, and changes its wording freely                                        |
| Keeping card data in archives        | The card would fetch it on demand, and an archive is text for the model to read; 64 KiB inline is enough for a card |
| An npm package for diffs             | The plugin's Rust code already parsed the diff into hunks, and a plugin cannot add a package without an SDK change  |

## Validation

M9:

- Registration tests of the slot, and tests of the `tool_call` renderer: an entry of the call's plugin is used, an entry of another plugin and a call without an entry show the generic card.
- Runtime tests: data and a failure travel from a fake plugin tool to the stored outcome and its view, data that is too large or not an object is dropped, and a record from before reads.
- The plugin Git's cards pass their Vitest tests and the end-to-end check with axe on WebView2.
