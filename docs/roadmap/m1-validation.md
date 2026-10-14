# M1 validation results

|              |                                                                                                                                                                                                                                               |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                                                                                                              |
| Last updated | 2026-10-14                                                                                                                                                                                                                                    |
| Related      | [Milestones](milestones.md#m1-chat), [M0 validation results](m0-validation.md), [Chat](../features/chat/README.md), [Window and tray](../features/window-and-tray.md), [ADR index](../adr/README.md), [Security](../architecture/security.md) |

This document records what the validations and exit criteria of [M1](milestones.md#m1-chat) showed so far, so that the owner can accept ADR-0011, ADR-0012, ADR-0020 and ADR-0021, and see what remains before v0.1.0. The checks stay in the repository and run in CI. Each section names the checks, where they ran, the result, and what is open.

The runs below were made on 2026-10-14 on the owner's Windows 11 workstation (WebView2 155.0.4283.45). Nothing in this document ran on Linux, in CI or against a live provider: the repository has no GitHub remote yet, and recording fixtures and the live runs need the owner's keys and endpoints. What is not run yet is marked so.

## 1. Summary

| Check or criterion             | Result                                                                                                  | Open                                                                                                                 |
| ------------------------------ | ------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| V8, provider conformance       | The adapters pass all 47 constructed fixtures; no fixture is recorded yet                               | Recording every server's fixtures; the nightly live workflow going green                                             |
| ADR-0012, durable runs         | Passed: runtime tests, crash tests and end-to-end tests on WebView2                                     | The end-to-end tests on WebKitGTK                                                                                    |
| ADR-0020, shell slots          | Passed: chat and window and tray are built through the slots alone                                      | None                                                                                                                 |
| ADR-0021, Markdown             | Passed: the hostile corpus through the renderer and through a provider; parse times within their bounds | The end-to-end tests on WebKitGTK                                                                                    |
| Reload, kill, Stop, retries    | Passed on WebView2                                                                                      | WebKitGTK                                                                                                            |
| Budgets and compaction         | Passed at the limits of every known model and at 4K, 8K, 32K and 128K tokens                            | None                                                                                                                 |
| First close, tray and quitting | Passed on WebView2 through the validation commands                                                      | The manual tray checks on Windows 11, a Linux desktop with tray support and GNOME without the AppIndicator extension |
| Performance targets            | All met on the workstation                                                                              | A run on the reference machine, which enforces them                                                                  |
| ADRs Accepted, v0.1.0 released | Not yet                                                                                                 | The owner's review, then the release                                                                                 |

## 2. V8: provider conformance

Checks: the conformance tests of `crates/mannie-provider` (`cargo test -p mannie-provider --features testing`), which replay each fixture through a local fixture server with random chunking and delays, and compare the normalized events with the fixture's reviewed expectation. `task provider:live` runs the same scenarios against a live server, and `task provider:record` records them.

Every fixture so far is constructed from the servers' documentation, and its manifest says so:

| Server     | Interface        | Constructed scenarios                                                                                                                                                                                                              | Recorded |
| ---------- | ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------- |
| OpenAI     | Responses        | text, length, reasoning, reasoning replay, overflow, unknown model, invalid key, rate limited (seconds and milliseconds), insufficient quota, server error, broken stream, CRLF line endings, an error in the middle of the stream | None     |
| OpenAI     | Chat Completions | text, length, overflow, unknown model                                                                                                                                                                                              | None     |
| OpenRouter | Chat Completions | text, length, reasoning, overflow, unknown model, an error in the middle of the stream                                                                                                                                             | None     |
| vLLM       | Chat Completions | text, length, reasoning, reasoning in think tags, overflow, unknown model                                                                                                                                                          | None     |
| SGLang     | Chat Completions | text, length, reasoning, overflow, unknown model, server error                                                                                                                                                                     | None     |
| LM Studio  | Chat Completions | text, length, reasoning, overflow, unknown model, broken stream                                                                                                                                                                    | None     |
| Ollama     | Chat Completions | text, length, reasoning, unknown model, a stream without `[DONE]`                                                                                                                                                                  | None     |

Result: all 47 constructed fixtures pass. V8 itself is not passed: it needs a recorded fixture of each core scenario on every server, and the nightly live run (`.github/workflows/provider-live.yml`) green.

Open, for the owner: record each server with `task provider:record` and the `MANNIE_LIVE_*` variables; give the repository the variables and secrets that `provider-live.yml` lists; review each recorded fixture's expectation. A quirk is trusted only once a recorded fixture shows it, so the detection probes for Ollama, LM Studio, vLLM and SGLang wait for these recordings too.

## 3. ADR-0012: runs as durable actors

Checks:

- `crates/mannie-runtime/tests/loop.rs`: every branch of a run on paused time, including retries, `Retry-After`, continuation notes, Stop within a second, permanent failures, encrypted reasoning per binding, and servers that cut requests silently.
- `crates/mannie-runtime/tests/subscriptions.rs`: a property test of appends, subscriptions and lagging subscribers in random orders, each of which ends with the final transcript and increasing sequence numbers.
- `crates/mannie-runtime/tests/crash.rs`: a child process killed after events 1, 3, 6, 8, 11, 13, 16 and 20 of a reply leaves an intact transcript, an interrupted run and a working retry.
- `tests/e2e/specs/chat-durability.test.ts`: a reload during a reply shows the whole reply without gaps; Stop takes effect in less than a second and keeps the text; a retry waits as long as `Retry-After` asks; a refused credential is not retried and shows its cause; a killed app leaves the reply interrupted, and Retry finishes it.

Result: all pass. The decision's two refinements, snapshots for new subscribers and the removal of a part's deltas once the part ends, are recorded in the ADR and covered by the property test.

## 4. ADR-0020 and ADR-0021: the shell's slots and Markdown

- The chat feature registers a view, a sidebar panel, an overlay, four part renderers, commands with availability, and two settings sections; window and tray registers an overlay and a settings section. Neither edits the shell (`tests/apps/desktop/src/lib/registrations/compose-shell.test.ts` composes the real feature list).
- The renderer's unit tests run the shared hostile corpus and check that only allowed node kinds come out and that no address reaches an attribute. `tests/e2e/specs/chat-security.test.ts` sends the same corpus from a provider through the real path: no script runs, no connection opens, no link or image element appears, and the Content Security Policy reports nothing.
- A link opens only after the dialog shows its normalized address, and a `javascript:` link is refused with its reason.
- Parsing stays within its bounds: texts within the cheap bounds parse on the main thread, and longer or denser texts in the worker, which gives up after 3 seconds and shows plain text.

## 5. End-to-end tests

`task e2e` on WebView2 runs 12 spec files: the M0 specs (shell, security, IPC, locale and startup problems) and those of M1 (chat, durability, security, compaction, providers, window and tray, and the pseudo-locale of the chat and window views). All pass, with axe finding no WCAG 2.1 AA violation in the chat views in the light and dark themes, in their dialogs and in the settings.

The run found and fixed these defects:

1. A command that runs on the main thread, `subscribe_window_requests`, panicked when it started its channel, because the channel's task needed a Tokio runtime that the main thread does not have. Channels now start on Tauri's runtime, and a unit test starts one without a runtime.
2. The conversation's profile menu stayed open after a profile was chosen, and with it the menu's lock on the rest of the page, because the registry's radio items keep their menu open by default. The menu now closes with the choice.
3. In the light theme, destructive buttons had a contrast of 4:1 between their text and their tint, below WCAG 2.1 AA. The theme's destructive color is darker in the light theme and lighter in the dark theme, with at least 5.2:1 in both.

The window of the validation build opens off every screen, without a taskbar button and without the focus, so the tests run without disturbing the owner's screen and keyboard ([desktop app](../architecture/desktop-app.md#7-window-and-lifecycle)).

Not run yet: the same tests on WebKitGTK, which the CI job runs on Ubuntu 22.04 with GNOME Keyring.

## 6. Performance

`task e2e:perf` measures the targets in the real app with a local provider. On the workstation:

| Target                                                                     | Measured | Target   |
| -------------------------------------------------------------------------- | -------- | -------- |
| NFR-PERF-01, a streamed piece appears (p95, from the provider to the page) | 88 ms    | 100 ms   |
| NFR-PERF-02, input with 8 replies streaming (p95, input to painted frame)  | 17 ms    | 50 ms    |
| NFR-PERF-03, a cold start shows a usable window                            | 1,267 ms | 2,000 ms |
| NFR-PERF-04, a conversation of 10,000 parts opens                          | 59 ms    | 1,000 ms |
| A reply of 32 KB at 200 pieces a second, input (p95)                       | 26 ms    | 50 ms    |

The first figure has the least margin: most of it is the runtime's 50 ms batching of deltas, which the provider-to-page measurement includes. The targets bind on the reference machine, which has not run the suite yet. CI records the figures without enforcing them.

## 7. Manual checks

Not done yet, and needed before v0.1.0: the tray on Windows 11, on a Linux desktop with tray support, and on GNOME without the AppIndicator extension, as the [window and tray](../features/window-and-tray.md#acceptance) acceptance lists; and a session with a real provider: sending, streaming, reloading, stopping, retrying, killing and restarting, compacting, links, the first close, the tray and quitting.

## 8. Decisions for the owner

- Accept, change or supersede ADR-0011, ADR-0012, ADR-0020 and ADR-0021.
- Review the defaults recorded in the [chat](../features/chat/README.md#decisions) and [window and tray](../features/window-and-tray.md#decisions) features, and the text that the model sees, which `crates/mannie-runtime/src/texts.rs` holds.
- Provide the provider keys and endpoints for V8, the GitHub remote with its secrets, and the release key pair.
