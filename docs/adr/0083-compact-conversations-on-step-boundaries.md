# ADR-0083: Compact conversations on step boundaries with calibrated budgets and visible markers

|          |                                                                                                                                                                                                                                                                                                                                        |
| -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D77)                                                                                                                                                                                                                                                                                                                         |
| Date     | 2026-11-27                                                                                                                                                                                                                                                                                                                             |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                  |
| Related  | [ADR-0002](0002-grow-through-extension-points.md), [ADR-0011](0011-own-the-provider-adapters.md), [ADR-0012](0012-run-agents-as-durable-actors.md), [ADR-0062](0062-let-the-owner-pay-for-model-calls-with-a-chatgpt-plan.md), [ADR-0072](0072-remove-tool-round-and-call-caps.md), [Conversations](../features/chat/conversations.md) |

## Context

- [ADR-0012](0012-run-agents-as-durable-actors.md), decisions 7 and 8, established provider-reported token counts as the truth for context budgets and compaction as a pure projection over complete turns.
- [ADR-0072](0072-remove-tool-round-and-call-caps.md) removed tool-round and tool-call caps so long agentic tasks can run to completion in a single turn.
- In long multi-step runs (such as VM installation, console wizards and browser workflows), several gaps in the turn-only compaction design combined to cause silent compaction while the UI showed roughly half the context used, and caused the model to lose fine-grained live state:
  - `older_results` pruned all but the newest three tool results of the active turn (`leave_out_results: true`), so the provider reported the stripped call size (~50% of budget) to the context meter and ledger. As soon as the next turn began, those results re-entered the unstripped request, spiking the projected context past 80% and triggering compaction before the first call of the new turn.
  - `estimate_message` counted encrypted reasoning payloads (`Content::ProviderItem`) as two tokens per five bytes of Base64 encrypted data, and `write_checkpoint` reset the calibration ledger to empty, causing raw overestimates after a checkpoint.
  - `turns_estimate` used a different formula from `estimate_request`, miscounting reasoning items, error parts, tool images and context excerpts; `tokens_after` logged the worst-case 15% summary allowance instead of the actual post-compaction size.
  - `plan` targeted 30% of the budget and kept at most `[2, 1, 0]` whole turns, frequently dropping to `keep = 0` and summarizing the turn that had just finished, while `render_turns` truncated every tool call's arguments and result to 500 characters from the start only.
  - Sliding-window `KEPT_RESULTS = 3` pruning mutated earlier messages on every step, invalidating prefix prompt caching and replacing pruned outputs with text that urged the model to run the tool again.
  - Proactive automatic compaction failures (such as `summary_cut` when a reasoning model spent its output limit thinking) aborted runs whose original request still fit within the budget.
  - The UI rendered only the single latest checkpoint below `covers_through_turn`, scrolled out of view or omitted when pagination loaded fewer turns, and showed no indicator when older tool results were pruned.

## Decision

1. Refine [ADR-0012](0012-run-agents-as-durable-actors.md), decision 8, to allow checkpoints on clean step boundaries inside a turn as well as on whole-turn boundaries:
   - A checkpoint continues to record `covers_through_turn` and gains an optional `covers_through_part` stored additively in its versioned JSON document and exposed additively on `CheckpointView`.
   - A whole-turn checkpoint leaves `covers_through_part` unset and hashes the exact covered turns as before, keeping every existing checkpoint valid.
   - A step-boundary checkpoint sets `covers_through_part` to the last part of a settled step (never splitting an `(run_id, attempt)` group, never splitting a tool call from its result and never covering a streaming background task part). Its hash chains the previous checkpoint's hash with the exact newly covered turns and part prefix.
   - When a checkpoint ends inside a turn, `build_request` precedes the remaining parts of that turn with a user anchor message carrying the turn's request (bounded when large) and a short note that earlier steps of the turn are summarized in the checkpoint, so every provider request still starts cleanly with the owner's intent.
   - Manual compaction uses the conversation's configured agent and reasoning effort, and can also compact settled earlier steps of a multi-step single turn.
2. Refine [ADR-0012](0012-run-agents-as-durable-actors.md), decision 7, to unify and calibrate context estimation:
   - Estimate `Content::ProviderItem` without counting Base64 encrypted payload bytes, using bounded reasoning overhead calibrated against provider-reported usage.
   - Carry the calibrated tokens-per-estimate ratio in the conversation ledger across checkpoints so the first call after compaction stays calibrated.
   - Measure candidate compaction savings with the same request-building projection used for model calls rather than a separate `turns_estimate` heuristic.
   - Compute `tokens_after` from the actual post-compaction request estimate (calibrated by the ledger) rather than the worst-case summary allowance.
   - On the ChatGPT plan ([ADR-0062](0062-let-the-owner-pay-for-model-calls-with-a-chatgpt-plan.md), decision 10), where requests omit `max_output_tokens`, cap the budget's output reserve to 32,768 tokens (at most the profile's output limit) while keeping the 5% safety margin and keeping the full `output_limit` reserve for standard endpoints that send `max_output_tokens`.
3. Improve compaction quality, recency preservation and resilience:
   - Raise the compaction target from 30% to 45% of the budget, cap `summary_allowance` at 8,192 tokens (between 1,024 and the profile's output limit), and retain recent turns or recent steps of a large turn up to a recency budget instead of dropping straight to `keep = 0`.
   - Give summary calls an adaptive per-call character limit (1,000 to 4,000 characters, higher for the newest covered turns) with head-and-tail truncation, ask for an explicit `In progress` section capturing live workflow state (step progress, values applied versus pending, open pages, VMs, hosts, branches and paths), and use the lowest explicit reasoning effort supported by the model so reasoning does not exhaust the summary limit.
   - Treat proactive automatic compaction failures as fail-soft when the request still fits the budget: log a warning and continue (falling back to tool-result pruning when tools are enabled). Forced overflow compactions and manual compactions continue to surface failures.
4. Make active-turn tool-result pruning cache-friendly and truthful:
   - Replace per-step sliding-window pruning with epoch-based watermark pruning: advance the pruned part watermark in a batch when projected context passes 80% and hold that watermark steady across later steps until context crosses 80% again, preserving prompt-prefix caching.
   - Replace pruned tool results with a compact factual stub naming the tool, key target arguments, size and any archive IDs rather than urging the model to re-run the tool.
   - Extend `ContextUsage` additively with `results_pruned` and `unpruned_tokens` (omitted when default so link documents of earlier versions stay identical), and log pruning activations and per-run `peak_input_tokens`, `cached_input_tokens` and `retries`.
5. Make compaction visible in the transcript and context meter, and classify payload overflows cleanly:
   - Extend `CheckpointView` additively with `covers_through_part` and `trigger`, and extend `ConversationSnapshot` additively with `checkpoints` (all checkpoints, oldest first) while keeping `checkpoint` as the latest checkpoint for backward compatibility ([ADR-0002](0002-grow-through-extension-points.md)).
   - Render markers for every visible checkpoint: after the covered turn for whole-turn checkpoints, inline after the covered part for step-boundary checkpoints, and as a top banner when the latest checkpoint sits above paginated earlier turns. Show the automatic compaction threshold and any active tool-result pruning in the context meter.
   - Classify HTTP `413` responses and broader token-limit refusal wording as `ProviderError::ContextOverflow`, and log redacted provider refusal metadata (`status`, `code`, `kind`, `param`) at `warn` level.

Earlier decisions that this ADR changes:

| Earlier decision                                                 | Change                                                                                                                                                              |
| ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [ADR-0012](0012-run-agents-as-durable-actors.md), decisions 7, 8 | Refined: checkpoints may cut on settled step boundaries inside a turn, calibration carries across checkpoints, and tool-result pruning reports unpruned usage state |

## Consequences

- Positive:
  - Long single-turn runs compact smoothly on step boundaries instead of stripping results silently, losing live state at the turn boundary or failing with unrecoverable overflows.
  - The context meter and transcript never hide automatic compactions or active result pruning.
  - Epoch-based pruning preserves prompt-prefix caching across consecutive tool steps.
  - Summary calls preserve exact in-progress state and stop failing healthy runs on proactive `summary_cut`.
- Negative:
  - `ConversationSnapshot`, `CheckpointView` and `ContextUsage` gain optional additive fields (all defaulted and skipped when empty/false to preserve worker link wire compatibility).
  - Step-boundary checkpoints add slicing logic to checkpoint verification and request assembly.
- Risks:
  - An invalid step boundary could split a tool call from its result or freeze a streaming background task into a hash. Boundary selection enforces settled `(run_id, attempt)` step boundaries with zero streaming or unsettled tool parts, covered by unit and integration tests.

## Alternatives considered

| Option                                             | Why not chosen                                                                                                            |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| Keep turn-only compaction and raise `KEPT_RESULTS` | Fails to solve uncapped single-turn growth under ADR-0072 and still wipes the finished turn on the next user message      |
| Add a SQLite column migration for part checkpoints | Unnecessary because `runtime_checkpoints.document` is already a versioned JSON column checked with `json_valid(document)` |
| Hard-fail runs on every automatic compaction error | Aborts healthy runs at 80% budget when the model still has headroom to finish or fall back to result pruning              |

## Validation

Runtime, store, provider, link-compatibility and desktop unit tests verify:

- step-boundary checkpoint planning, hash chaining, backward compatibility with whole-turn checkpoints, and anchor injection in `build_request`;
- `ProviderItem` estimation, preserved calibration across checkpoints, actual `tokens_after` measurement, ChatGPT plan output reserve, fail-soft proactive compaction, lowest-effort summary calls, manual compaction using the conversation's agent, and epoch-based cache-stable pruning;
- HTTP `413` and expanded overflow classification in `mannie-provider`;
- additive JSON wire compatibility across all link document versions in `mannie-link`;
- multi-checkpoint and pruned-context reducer and UI state handling in Vitest and `svelte-check`.
