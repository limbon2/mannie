# Providers

|              |                                                                                                                                                                                                                                                                             |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                    |
| Milestone    | M1                                                                                                                                                                                                                                                                          |
| Last updated | 2026-10-14                                                                                                                                                                                                                                                                  |
| Part of      | [Chat](README.md)                                                                                                                                                                                                                                                           |
| Related      | [ADR-0011](../../adr/0011-own-the-provider-adapters.md), [ADR-0010](../../adr/0010-keep-provider-credentials-on-the-master.md), [Agent runtime](../../architecture/agent-runtime.md#4-model-calls), [Security](../../architecture/security.md#7-rules-for-handling-secrets) |

## Design

### Provider profiles

A profile names one model on one server. The core validates every field; the webview only sends the owner's input.

| Field            | Rule                                                                                                                                                                             |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Name             | 1 to 100 characters after trimming, unique among profiles regardless of case                                                                                                     |
| API interface    | Responses or Chat Completions                                                                                                                                                    |
| Base URL         | An absolute `http` or `https` URL of at most 2,048 characters, without user information, query or fragment. Mannie appends `/responses`, `/chat/completions` or `/models`.       |
| Model            | 1 to 200 characters without control characters                                                                                                                                   |
| Credential       | Optional, because local servers often need none. Stored only in the operating system's credential store ([ADR-0010](../../adr/0010-keep-provider-credentials-on-the-master.md)). |
| Context window   | 1,024 to 10,000,000 tokens                                                                                                                                                       |
| Output limit     | At least 16 tokens, and small enough that the context budget keeps at least 1,024 tokens ([conversations](conversations.md#context-budget-and-compaction))                       |
| Reasoning effort | The server's default, or one of the efforts the API defines. The server's default sends no effort at all.                                                                        |

- Known models have defaults for the context window and the output limit, kept with their verification dates in `crates/mannie-provider/src/models.rs`. When the server reports a model's limits, the provider test shows them, and the owner can apply them.
- One profile is the default. A conversation uses the default unless the owner chose another profile for it. Deleting the default profile makes the oldest remaining profile the default. A profile that a running reply uses cannot be deleted until the reply ends; conversations that used a deleted profile fall back to the default.
- Each profile has a configuration revision that increases when the API interface, the base URL, the model or the credential changes. Provider-private data, such as encrypted reasoning, is bound to the profile and its revision and replayed only to that binding ([ADR-0011](../../adr/0011-own-the-provider-adapters.md), decision 6).

### Credentials

- The credential store entry is named `provider/<profile id>/<revision>` under the bundle identifier. Replacing a credential writes the new entry, updates the profile, and then deletes the old entry, so a failure in between never loses the working credential.
- The database holds only the entry's name. The webview sends a credential once, in the command that creates or updates a profile, and never receives it back: the profile list says only whether a credential is stored.
- When the core reads a credential for a reply or a test, it registers the value with the log redactor, keeps it in the redacting secret type, and drops it when the reply ends. Requests mark the authorization header as sensitive.
- A credential travels over plain HTTP only to a loopback address or a private network address written as an IP address. For other hosts the profile needs `https`.
- Without a credential store, such as on a Linux desktop without a Secret Service, Mannie refuses to save a credential and explains how to enable one (V7). Profiles without a credential still work.

### The provider test

The test runs these checks in order and reports each one as passed, failed with a cause, skipped or worth a warning:

| Check      | What it does                                                                                                            |
| ---------- | ----------------------------------------------------------------------------------------------------------------------- |
| Credential | Reads the stored credential, or skips when the profile has none                                                         |
| Server     | Lists the models at `/models` and identifies the server. Fails on an unreachable server or a refused credential.        |
| Model      | Checks that the server lists the model, and reports the context window and output limit the server gives for it, if any |
| Reply      | Streams a short request ("Reply with the single word: ready") and checks that text and a finish reason arrive           |
| Usage      | Checks that the server reported token counts, which the context budget relies on                                        |
| Reasoning  | When the profile sets a reasoning effort, checks that the server accepted it and reports whether reasoning arrived      |

### Adapters

- `mannie-provider` defines the `Provider` trait: one streamed model call that yields normalized events (text and reasoning deltas, provider-private reasoning items, usage, and the end of the step with its reason). There is one adapter per API interface, registered in the provider registry.
- Transport: reqwest with rustls and ring, and the operating system's trust store through `rustls-platform-verifier`. Redirects are refused, so a credential never follows one. Timeouts: 15 s to connect, 120 s for the response headers, 600 s for the first event (local servers can take minutes to read a long prompt), and 300 s between events. One event may be at most 1 MiB, and one response at most 32 MiB.
- Responses API: `stream: true`, `store: false`, the owner's instructions as `instructions`, `max_output_tokens`, and, with a reasoning effort, `reasoning: {effort, summary: "auto"}` and `include: ["reasoning.encrypted_content"]`.
- Chat Completions: `stream: true`, `stream_options: {include_usage: true}`, the instructions as the first `system` message, and `max_completion_tokens`, or whatever the quirk profile says instead. Reasoning arrives as `reasoning` or `reasoning_content` in the deltas, or, for some models, between `<think>` tags at the start of the content.
- Errors fall into three classes ([ADR-0011](../../adr/0011-own-the-provider-adapters.md), decision 3):

  | Class            | Cases                                                                                                                                                                    |
  | ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
  | Transient        | HTTP 408, 409, 425, 429 and 5xx, connection failures, timeouts and broken streams. `Retry-After` and `retry-after-ms` are honored.                                       |
  | Context overflow | The error codes and messages that each server uses when the request is longer than the model's context                                                                   |
  | Permanent        | HTTP 401 (invalid credential), 402 and 429 with `insufficient_quota` (quota), 403 (permission), 404 (model or endpoint), other 4xx, refused redirects, malformed streams |

- Each stream's integrity is checked: the content type is `text/event-stream`, a finish reason or completion event arrives, and events stay within their size limits. A stream that ends without one is a broken stream.

### Quirk profiles

Quirk profiles are data in `crates/mannie-provider/src/quirks.rs`. Each one matches a server, a model or both, and switches a documented normalization on. A quirk is trusted only when a recorded fixture shows the behavior.

| Quirk                       | Effect                                                                                                                                                                                                                                                                                     |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Reasoning in `<think>` tags | Text between `<think>` and `</think>` at the start of the content becomes reasoning                                                                                                                                                                                                        |
| No stream options           | `stream_options` is left out, and the context budget estimates usage                                                                                                                                                                                                                       |
| Output limit field          | `max_tokens` replaces `max_completion_tokens`                                                                                                                                                                                                                                              |
| Reasoning effort shape      | `reasoning_effort`, `reasoning: {effort}`, or no effort at all                                                                                                                                                                                                                             |
| Context overflow messages   | The server's wording of a context overflow, for servers without an error code                                                                                                                                                                                                              |
| Silent truncation           | The server shortens prompts beyond its context instead of failing. Between two calls of a conversation the request only grows, so when the reported size grows by less than half of what was added, the reply's status says that the model may not have read the start of the conversation |

The server is identified by the base URL's host for OpenAI and OpenRouter, and by probing endpoints that only one server has for Ollama, LM Studio, vLLM and SGLang. An unidentified server gets the generic Chat Completions behavior.

Ollama serves a model with the context size of its `num_ctx` setting, which is smaller than most models' windows unless the owner raises it, and cuts longer requests without an error. A profile for Ollama should therefore use `num_ctx` as its context window. The provider test cannot read `num_ctx`, so the silent-truncation check above is the safety net.

### Contracts

- Commands (`apps/desktop/src-tauri/src/commands/providers.rs`): `list_provider_profiles`, `create_provider_profile`, `update_provider_profile` (the credential is kept, replaced or removed), `delete_provider_profile`, `set_default_provider_profile`, `test_provider_profile`, and `get_model_defaults` for the known limits of a model name.
- Errors use the `ProviderProfileError` codes, such as `name_taken`, `invalid_base_url`, `insecure_credential_transport`, `invalid_limits`, `credential_store_unavailable` (with its reason), `profile_in_use` and `not_found`. Failures of a reply use the run failure codes in [conversations](conversations.md#errors).

### Data

`provider_profiles` (migration `0006`): the profile's fields, the credential entry's name, whether it is the default (at most one row), the configuration revision, and a versioned JSON document with the limits, the reasoning effort and what the provider test found out about the server.

### Recorded fixtures (V8)

- Fixtures live in `crates/mannie-provider/tests/fixtures/<server>/<scenario>/`: the request Mannie sent, the response status with its content type and retry headers, the raw stream (`body.sse`), the normalized events the adapter must produce, and a manifest with the server and its version, the model, the date, and the provenance: recorded from a live server, or constructed for cases that a live server does not produce on demand.
- Recorded scenarios on every server: a short reply, a reply cut at the output limit, the model list, an unknown model, an invalid credential where the server checks credentials, a context overflow, and reasoning where the model supports it. The Responses API adds a two-turn exchange that replays encrypted reasoning.
- Constructed scenarios: rate limits with `Retry-After`, server errors, an exhausted quota, broken streams, CRLF line endings, keepalive comments, an error in the middle of a stream, an oversized event and malformed chunks.
- A fixture server in the tests replays each stream with random chunk boundaries and delays. `task provider:record` records fixtures from the owner's servers and refuses to write a file that contains the credential. `task provider:live` runs the same scenarios against live servers, and a nightly workflow runs it with the repository's provider secrets.

### Message keys

Settings and errors of providers use keys under `chat_providers_` and `chat_error_`.

### Threat notes

- Credentials (T-6): only in the credential store; registered with the redactor; never in the database, logs, URLs, the webview's responses or fixtures; never sent over plain HTTP to a remote host; never forwarded by a redirect. A planted credential is searched for in every log, database and crash file of the end-to-end tests.
- Provider output is untrusted. Size limits bound memory, the parser rejects malformed input without panicking, and the text reaches the webview only through the Markdown renderer of [conversation view](conversation-view.md#markdown-rendering).
- A compromised webview can create a profile that points anywhere and send a conversation there. This is part of the residual risk of [security](../../architecture/security.md#8-if-the-webview-is-compromised): the webview can call every command the main window is granted.

### Platform differences

| Topic            | Windows                       | Linux                                                                                     |
| ---------------- | ----------------------------- | ----------------------------------------------------------------------------------------- |
| Credential store | Windows Credential Manager    | Secret Service (GNOME Keyring or KWallet). Without one, credentials cannot be saved (V7). |
| Trust store      | The Windows certificate store | The distribution's CA bundle                                                              |

## Acceptance

- V8: the conformance tests pass with recorded fixtures of OpenAI's Responses API and of Chat Completions on vLLM, SGLang, LM Studio, Ollama and OpenRouter, and the nightly live run passes. Constructed fixtures cover the error cases and are marked as constructed.
- The SSE parser yields the same events for any split of a stream into chunks, and never panics on arbitrary bytes (property tests).
- Every row of the error classification has a test, including `Retry-After` in seconds and as a date, and `insufficient_quota`.
- Creating, editing, testing, deleting and choosing the default profile work from the provider settings page (end-to-end tests against the mock provider).
- A planted credential appears in no log, database or crash file after the end-to-end scenario, and never in a recorded fixture.
- The dependency graph contains ring as the only crypto provider and no OpenSSL or aws-lc (`task lint:deps`).
