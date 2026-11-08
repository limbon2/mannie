# M19 validation results

|              |                                                                                                                                                                                                                                                                  |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                                                                                                                                 |
| Last updated | 2026-11-09                                                                                                                                                                                                                                                       |
| Related      | [Milestones](milestones.md#m19-chatgpt-plan), [ADR-0062](../adr/0062-let-the-owner-pay-for-model-calls-with-a-chatgpt-plan.md), [ChatGPT plan](../features/chatgpt-plan.md), [Providers](../features/chat/providers.md), [Security](../architecture/security.md) |

This document records what the checks of [M19](milestones.md#m19-chatgpt-plan) showed, so that the owner can review the ChatGPT plan and ADR-0062, and see what remains before the release. The owner accepted ADR-0062 with the plan (D51). Each section names the checks, where they run, the result, and what is open. The automated checks passed on 2026-11-08. The first use with the owner's ChatGPT account, on 2026-11-09, found two differences of the live route, which are fixed (section 6); its other checks are pending.

## 1. Summary

| Check                                                          | Result  | Open                                               |
| -------------------------------------------------------------- | ------- | -------------------------------------------------- |
| The live route, `task provider:live:chatgpt`                   | Pending | Every scenario of section 2                        |
| Recorded fixtures in place of the constructed `open_ai/plan_*` | Pending | The recording, with `task provider:record:chatgpt` |
| The development build on Windows                               | Started | The checks of section 3 that are pending           |
| The UI pass                                                    | Pending | Every check of section 4                           |
| `task ci`                                                      | Passed  | None: 14 steps passed on Windows on 2026-11-08     |
| The end-to-end test against the mock of OpenAI                 | Passed  | None on WebView2; WebKitGTK is open (section 5)    |

## 2. The live route

`task provider:live:chatgpt` runs the ignored test `crates/mannie-provider/tests/live_chatgpt.rs` against OpenAI's real route:

- The test prints the sign-in URL, and the owner signs in with their own ChatGPT account in a browser. Nobody else types a credential. The first sign-in needs port 1455 free.
- `MANNIE_LIVE_CHATGPT_MODEL` can name a model's slug, which is otherwise the first model of the account's catalog, and `MANNIE_LIVE_REASONING_EFFORT` an effort, which is otherwise `low`.
- It keeps the tokens in memory only, and revokes them at its end.
- `task provider:record:chatgpt` records the scenarios as fixtures and refuses to write a file that holds a token. The recorded fixtures replace the constructed ones.

| Scenario                          | What it shows                                                                                                                             | Result  |
| --------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | ------- |
| Text                              | A short reply on the plan route, from a request without `max_output_tokens`                                                               | Pending |
| Tool call                         | A call of a function in the namespace `mannie`, and the answer after its result                                                           | Pending |
| Replay                            | A second turn that replays the call with `namespace`, the assistant message with `phase`, and encrypted reasoning through `include`       | Pending |
| Flat tools                        | Whether the route also takes function tools outside a namespace                                                                           | Pending |
| A tool missing from the namespace | Whether the route takes a replayed call of a tool that the namespace no longer holds. If it refuses it, Mannie renders such calls as text | Pending |
| Refresh                           | A refresh returns a new refresh token, and the rotated one is refused afterwards                                                          | Pending |
| Revocation                        | The revocation answers an empty HTTP 200, and the revoked refresh token is refused afterwards                                             | Pending |
| The port of a registration        | Whether a registration takes a port other than 1455, which OpenAI's example uses                                                          | Pending |
| Errors                            | The codes of plan usage, recorded when the route returns them, and constructed otherwise                                                  | Pending |

## 3. The development build on Windows

| Check                                                                         | Result                                                  |
| ----------------------------------------------------------------------------- | ------------------------------------------------------- |
| Sign in with the owner's ChatGPT account                                      | Passed on 2026-11-09                                    |
| Choose a model from the account's catalog                                     | Passed on 2026-11-09, after the first fix of section 6  |
| A reply on a plan profile, with GPT-6.1 Sol at the effort `max`               | Passed on 2026-11-09, after the second fix of section 6 |
| Run the provider test on a plan profile                                       | Pending                                                 |
| A conversation with tool calls whose encrypted reasoning replays across turns | Pending                                                 |
| Two child agents at once on a plan profile                                    | Pending                                                 |
| A restart, then a reply without a new sign-in                                 | Pending                                                 |
| Sign out, and confirm that OpenAI revoked the session                         | Pending                                                 |

## 4. The UI pass

| Check                                                                                                                                                       | Result  |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------- | ------- |
| The account's item, the notice, the profile dialog's plan fields, the indicator and the actions of a failed reply in WebView2, in the light and dark themes | Pending |
| Every action from the keyboard only                                                                                                                         | Pending |
| The pseudo-locale, with every text from the catalog and no layout that breaks                                                                               | Pending |
| Axe, with no WCAG 2.1 AA violation                                                                                                                          | Pending |

## 5. The end-to-end test

`tests/e2e/specs/chatgpt-plan.test.ts` drives the validation build against the provider server's mock of OpenAI, which signs its ID tokens with a key of its own. On Windows with WebView2, on 2026-11-08, all six steps passed:

- a sign-in through the recorded authorize page, the redirect to Mannie's listener, the code exchange and the key set, then the notice and "Got it";
- a profile on the plan from the catalog;
- a reply without `max_output_tokens`, with the indicator above the message box;
- a refresh of the tokens before a call, after the access token expired;
- the usage limit, with "Manage usage" opening ChatGPT's usage settings;
- a sign-out that revokes the refresh token and keeps the registration.

## 6. Findings on the owner's machine

The first use of the development build with the owner's ChatGPT account, on 2026-11-09, found two ways in which the live route differs from what Mannie expected. Both made Mannie report an answer that it could not read (`invalid_response`), and both are fixed:

- The catalog of `GET /v1/models` is longer than 64 KiB. Mannie read it with the reader of error bodies, which keeps the first 64 KiB, so the JSON was cut and did not parse, and the profile dialog could not list the models. Mannie now reads the whole catalog, up to the 16 MiB of other lists of models ([ADR-0062](../adr/0062-let-the-owner-pay-for-model-calls-with-a-chatgpt-plan.md), decision 11). A test serves a catalog more than four times as long, in small chunks, and fails with the old reader.
- `POST /v1/responses` answers with an event stream that has no content type. Mannie refused every reply as an answer without an event stream: the log showed status 200, 64 KiB within two seconds, and no content type. On the plan, Mannie now reads such an answer as its event stream, and still refuses an answer that names another type (ADR-0062, decision 10, which refines [ADR-0011](../adr/0011-own-the-provider-adapters.md), decision 4). The constructed fixtures of the plan's streams now have no content type.

The log now describes an answer that Mannie cannot read, at the level `warn`: its status, content type, size and error code, or for an event, its name, its size and where its JSON breaks. These lines never hold the answer's text, which may hold the owner's content.

## 7. Open

- Every check above.
- Linux: the Secret Service with the split tokens, and the end-to-end test on WebKitGTK.
