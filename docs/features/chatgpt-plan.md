# ChatGPT plan

|              |                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Milestone    | M19                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Last updated | 2026-11-09                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Related      | [ADR-0062](../adr/0062-let-the-owner-pay-for-model-calls-with-a-chatgpt-plan.md), [Providers](chat/providers.md), [Conversation view](chat/conversation-view.md), [ADR-0010](../adr/0010-keep-provider-credentials-on-the-master.md), [ADR-0011](../adr/0011-own-the-provider-adapters.md), [Credential leases](../../plugins/workers/docs/credential-leases.md), [Security](../architecture/security.md), [Feature protocol](README.md) |

## Intent

The owner wants Mannie's conversations to run on their ChatGPT subscription instead of a paid API key: "i want to introduce support for a chatgpt subscription". OpenAI's Sign in with ChatGPT lets open-source apps that run locally call its Responses API with tokens that bill the user's ChatGPT plan ([ADR-0062](../adr/0062-let-the-owner-pay-for-model-calls-with-a-chatgpt-plan.md)). Mannie signs in to one ChatGPT account, and the provider profiles that use the ChatGPT plan call OpenAI's models with its tokens, on the master only.

## Behavior

### The ChatGPT account

- Settings > Providers shows the ChatGPT account above the provider profiles, in one of these states:
  - loading;
  - signed out, with OpenAI's button "Continue with ChatGPT" and the ChatGPT logo, and "Use another account" once Mannie knows an account from an earlier sign-in;
  - signing in, while Mannie waits for the browser, with "Open the sign-in page again" and "Cancel";
  - signed in, with "Signed in as" and the account's email, "Manage usage", which opens ChatGPT's usage settings in the browser, and "Sign out";
  - sign-in needed, after the account's session ended, for example because its refresh token expired or OpenAI revoked it, with "Continue with ChatGPT";
  - the error of the last sign-in, which says what failed and what the owner can do.
- "Continue with ChatGPT" opens OpenAI's sign-in page in the system browser, where the owner signs in and allows Mannie to use the ChatGPT plan. The first sign-in registers Mannie with the account, under a name that the owner confirms with "Mannie" suggested, and asks the owner to allow what Mannie asks for. A later sign-in shows OpenAI's choice of account and asks no consent again. The browser's page then says that the owner can go back to Mannie.
- A sign-in waits 5 minutes for the browser. It fails, and says what the owner can do, when:
  - another program uses port 1455, which the first sign-in needs, such as the Codex CLI while it signs in. Nothing opens, and the message names the port;
  - the browser does not come back in time;
  - the owner declines in the browser;
  - OpenAI does not complete Mannie's registration, or refuses to redeem the sign-in;
  - the account's ID token fails a check;
  - the account does not allow Mannie to use its plan, and Mannie then revokes the tokens;
  - the credential store cannot be used;
  - OpenAI cannot be reached, or the system browser cannot be opened;
  - the owner cancels it.
- After the first sign-in, a notice shows once: "You're using your ChatGPT plan", "Eligible usage in this app uses your ChatGPT plan. Manage usage in your ChatGPT settings." and "Got it".
- Mannie holds one ChatGPT account. "Use another account" signs in to a new account, which then serves every plan profile in place of the old one.
- "Sign out" asks OpenAI to end the session and forgets the tokens. When OpenAI does not confirm, Mannie still forgets them, and says that the owner can disconnect Mannie in ChatGPT's settings. Plan profiles stay, and their replies fail until the owner signs in again. Mannie keeps its registration with the account, so the next sign-in asks no consent.
- The journal records no sign-in or sign-out, since they change only Mannie's own credentials, as a change of a profile's key does.

### Plan profiles

- The profile dialog asks how the profile reaches its model: "API endpoint", as before, or "ChatGPT plan".
- A plan profile has no fields for the API interface, the base URL or the API key. It calls OpenAI's Responses API at `https://api.openai.com/v1` with the account's tokens. The owner picks the model from the account's catalog by its name, and the dialog shows the catalog's loading, empty, error and signed-out states. The context window and the output limit come from the defaults of known models, or the owner enters them.
- The list of profiles marks plan profiles with "ChatGPT plan".
- Switching a profile to the ChatGPT plan forgets its API key.
- The provider test of a plan profile reads the account's catalog in place of the server's `/models`, and runs its other checks with the plan's requests ([providers](chat/providers.md#the-provider-test)).
- On the plan, the output limit is a budget that Mannie keeps: OpenAI's plan route takes no output limit, so a summary longer than its allowance is cut down.

### In conversations

- While the profile of a conversation uses the ChatGPT plan, "Using ChatGPT plan" shows beside the profile menu, with "Manage usage". A new conversation shows it above the message box while the default profile uses the plan. While the account is signed out or needs a sign-in, "Sign in to ChatGPT" shows there instead, which opens the Providers settings.
- Mannie refreshes the account's tokens by itself, about once an hour. A reply goes on across a refresh, replies and child agents that need a refresh at the same time share one, and the account stays signed in across restarts.
- When the plan's usage limit is reached, the reply fails with "Usage limit reached", "Review your plan or this app's limit in ChatGPT settings.", "Manage usage" as the first action, and Retry. Mannie never retries it by itself.
- When the account must sign in again, or another account signed in during the reply, the failed reply offers "Open ChatGPT settings", which opens the ChatGPT account in the Providers settings.
- When OpenAI cannot serve plan usage for a moment, Mannie retries as it does for other transient failures. A reply also fails, with its cause, when the account may not use its plan in other apps, or when the request holds something that the plan does not support.
- The encrypted reasoning of a reply replays only to the same profile, revision and account: a refresh, or a new sign-in of the same account, keeps it, and another account never receives it.

### Workers and plugins

- Plan profiles stay on the master. A run on a plan profile is offered no workers or other executors, and its follow-ups on tasks of executors are refused. Child agents on this computer work as before.
- Mannie never lends the account's tokens, and plugins never see the account.

### What this version does not do

- Hold several ChatGPT accounts.
- Lend the plan's tokens to workers.
- Embed or rerank on the plan.
- Use OpenAI's hosted tools, such as web search or file search.
- Call the models over WebSocket.
- Use the Codex app-server.

## Fit

- [Provider adapters and quirk profiles](../architecture/extension-points.md#33-provider-adapters-and-quirk-profiles): the plan's request shape and errors, as data of the Responses adapter that only plan profiles select ([ADR-0062](../adr/0062-let-the-owner-pay-for-model-calls-with-a-chatgpt-plan.md), decisions 10 and 12).
- [Transcript parts and run events](../architecture/extension-points.md#35-transcript-parts-and-run-events): six run failures, by addition.
- [IPC commands and channels](../architecture/extension-points.md#36-ipc-commands-and-channels) and [message catalogs](../architecture/extension-points.md#311-message-catalogs): the commands below, the account's channel, and the prefixes `chat_chatgpt_` and `chat_failure_chatgpt_`.
- [Store tables and migrations](../architecture/extension-points.md#39-store-tables-and-migrations): the table `provider_chatgpt_account`, and the field `access` of a profile's document.
- Chat's Providers settings and conversation view: the account's item, the profile dialog's choice, the indicator beside the profile menu and the actions of a failed reply.
- The credential store: entries under the core's prefix `provider/`, which no plugin can take.
- Must not change: the requests, credentials and bindings of endpoint profiles; the journal; the plugins and the plugin SDK, which need no change; the Workers link.
- Needs [ADR-0062](../adr/0062-let-the-owner-pay-for-model-calls-with-a-chatgpt-plan.md), which refines [ADR-0010](../adr/0010-keep-provider-credentials-on-the-master.md) and [ADR-0011](../adr/0011-own-the-provider-adapters.md), and follows [ADR-0004](../adr/0004-keep-system-access-in-the-rust-core.md) and [ADR-0019](../adr/0019-keep-visible-text-in-message-catalogs.md).

## Open questions

None blocks the build. The fixtures recorded from the live route settle OpenAI's quirks, which the constructed fixtures assume until then:

- whether the route also takes function tools outside a namespace;
- whether it takes `phase` on replayed assistant messages, and `include` of encrypted reasoning;
- how it takes a replayed call of a tool that the namespace no longer holds. If it refuses such calls, Mannie renders them as text;
- whether a registration takes a port other than 1455.

## Decisions

- 2026-11-08: the owner's answers ([D51](../roadmap/milestones.md#5-decisions)): plan profiles stay on the master, workers are not offered to their runs, and the core never lends plan tokens; the sign-in button is OpenAI's ChatGPT logo with "Continue with ChatGPT"; Mannie holds one ChatGPT account.
- 2026-11-08: the owner accepted ADR-0062 by approving the plan, with its defaults: the feature is in the core; OAuth written by hand after OpenAI's documentation; the tokens only in the credential store; one refresh at a time; a fresh token for every model call; no journal records of signing in and out; and several accounts, lending to workers, embeddings and reranking on the plan, hosted tools, WebSocket and the Codex app-server left for later.

## Design

### The sign-in

1. Mannie creates the host ID once and keeps it: `urn:uuid:` followed by a UUIDv4 from random bytes.
2. It binds the callback listener on `127.0.0.1`: on port 1455 for a first sign-in and for "Use another account", and on any free port otherwise. When port 1455 is busy, the sign-in fails with `port_in_use`, and nothing opens.
3. It builds the authorize URL with PKCE S256, a fresh `state` and `nonce`, the scopes `openid profile email offline_access resource.invoke chatgpt.tokens.use.direct`, `resource=https://api.openai.com/v1` and `ext_agent_host_id`. A first sign-in sends `client_id=dynamic_agent_client` and `agent_name_hint=Mannie`, and a later one sends the saved `client_id` and the saved email as `login_hint`.
4. `start_chatgpt_sign_in` asks the runtime for the sign-in, then opens the URL in the system browser through the core's link opener. The webview never receives the URL.
5. The listener answers only `GET /auth/callback`, gives each request 10 seconds and 16 KiB, and waits at most 5 minutes. A wrong `state` gets a 400 and the wait goes on, and an `error` parameter ends the sign-in. The callback of a first sign-in must carry the issued `client_id`, which Mannie keeps with the account once the sign-in completes.
6. Mannie redeems the code, checks the ID token and the granted `scope`, stores the tokens and records the account's subject and email. It checks the ID token's RS256 signature against OpenAI's keys, `iss`, `aud` against the client ID, `exp` and `iat` with 5 seconds of leeway, the `nonce` and a `sub` that is not empty, and then drops the ID token.

### Refreshes and calls

- A refresh starts when less than 5 minutes of the access token remain. One runs at a time, in a task that holds the account's gate until the rotated tokens are stored, and quitting waits up to 5 seconds for it.
- The refresh codes `invalid_grant`, `invalid_refresh_token`, `token_expired`, `refresh_token_expired`, `refresh_token_invalidated` and `refresh_token_reused` end the session: the state becomes `sign_in_needed`, and the token parts are deleted. Network failures, timeouts, 429 and 5xx keep the tokens.
- Each model call of a plan profile resolves its token right before it is made: each step, each retry, the summaries and compaction, the provider test and the catalog. A refused token leads to one refresh and one repeat, and a second refusal ends the run with `chatgpt_sign_in_needed`. A refused call repeats no tool operation.
- Signing out waits for a refresh in flight, revokes the refresh token with retries, and clears the tokens also when OpenAI does not confirm, which `SignOutReport.revoked` reports.

### Contracts

- `ProfileAccess`, the field `access` of `ProviderProfile` and `ProfileDraft`: `endpoint`, the default, which a draft without the field reads as, or `chatgpt_plan`. A plan profile's API interface, Responses, and its base URL come from the core's configuration and never from the webview, and a plan profile refuses a credential.
- Commands, in `apps/desktop/core/src/commands/chatgpt.rs`:

  | Command                       | What it does                                                                                                                                                    |
  | ----------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
  | `subscribe_chatgpt_account`   | Sends `ChatGptAccountUpdate`s on a channel: the account's `ChatGptAccountView` at once and after each change, and `ended` when Mannie quits                     |
  | `start_chatgpt_sign_in`       | Starts a sign-in from a `StartChatGptSignIn`, with `anotherAccount` and the texts of the browser's page, then opens OpenAI's sign-in page in the system browser |
  | `cancel_chatgpt_sign_in`      | Ends the sign-in that waits                                                                                                                                     |
  | `reopen_chatgpt_sign_in_page` | Opens the waiting sign-in's page in the browser again                                                                                                           |
  | `sign_out_chatgpt`            | Signs out, and returns a `SignOutReport` whose `revoked` says whether OpenAI confirmed the revocation                                                           |
  | `list_chatgpt_models`         | Returns the account's catalog as `ChatGptModel`s, each with its `slug` and `displayName`, in the catalog's order                                                |
  | `acknowledge_chatgpt_notice`  | Records that the owner saw the notice                                                                                                                           |

- Validation builds only: `validation_set_chatgpt_mock` points the sign-in and the API at the end-to-end tests' mock of OpenAI, with the first sign-in on any free port, and `validation_expire_chatgpt_token` makes the access token expire, so that a test crosses a refresh.
- `ChatGptAccountView`: `state`, which is `signed_out`, `signed_in` or `sign_in_needed`; `email`; `known`; `noticeSeen`; and `signIn`, the progress of a sign-in, which is `idle`, `waiting_for_browser`, `finishing`, or `failed` with its error.
- `ChatGptError`, whose codes for a failed sign-in are `port_in_use` (with the port), `timed_out`, `access_denied`, `registration_incomplete`, `sign_in_refused`, `id_token_invalid`, `plan_usage_not_granted`, `credential_store_unavailable` (with its reason), `network`, `browser_unavailable`, `canceled` and `already_signing_in`. Its other codes belong to the commands: `not_signing_in`, `invalid_page` (a text of the browser's page is empty, too long or has control characters), `signed_out`, `provider` (a call to OpenAI failed, with its run failure), `storage_failed` and `store_unavailable`.
- Run failures, by addition. [ADR-0062](../adr/0062-let-the-owner-pay-for-model-calls-with-a-chatgpt-plan.md), decision 12, maps OpenAI's codes to them:

  | Failure                             | Cause                                                                                                                                                 |
  | ----------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
  | `chatgpt_sign_in_needed`            | The account is signed out or its session ended, a token was refused again after a refresh, or OpenAI's permission context does not authorize the call |
  | `chatgpt_account_changed`           | Another account signed in during the reply                                                                                                            |
  | `chatgpt_usage_limit_reached`       | The plan's usage limit, or a limit of OpenAI's for Mannie, is reached. Never retried                                                                  |
  | `chatgpt_unavailable`               | OpenAI cannot serve plan usage for now. Retried with backoff, as other transient failures are                                                         |
  | `chatgpt_not_eligible`              | The account may not use its plan in other apps                                                                                                        |
  | `chatgpt_unsupported`, with `param` | The plan does not support part of the request, which `param` names in at most 64 safe characters                                                      |

### The text the model sees

A plan profile's request is the Responses API's ([providers](chat/providers.md#adapters)) apart from these points ([ADR-0062](../adr/0062-let-the-owner-pay-for-model-calls-with-a-chatgpt-plan.md), decision 10):

| Part of the request          | On the ChatGPT plan                                                                                                                     |
| ---------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| The tools                    | One tool `{"type": "namespace", "name": "mannie", ...}` holds every function, under the name it has elsewhere                           |
| A replayed tool call         | A `function_call` with `"namespace": "mannie"`                                                                                          |
| A replayed assistant message | `"type": "message"` and a `phase`: `commentary` when a tool call followed its text in the same model call, and `final_answer` otherwise |
| `max_output_tokens`          | Never sent                                                                                                                              |

The route answers with an event stream that has no content type. Mannie reads such an answer from this route alone, and still refuses one that names another type.

### Limits

| What                                         | Limit                                               |
| -------------------------------------------- | --------------------------------------------------- |
| A sign-in's wait for the browser             | 5 minutes                                           |
| A request to the callback listener           | 10 seconds and 16 KiB                               |
| The start of a refresh                       | When less than 5 minutes of the access token remain |
| The ID token's leeway for `exp` and `iat`    | 5 seconds                                           |
| Quitting's wait for a refresh in flight      | 5 seconds                                           |
| A part of the tokens in the credential store | 1,000 UTF-16 units                                  |
| The parts of the tokens                      | 32                                                  |
| The `param` of `chatgpt_unsupported`         | 64 safe characters                                  |
| The account's catalog                        | 16 MiB, as other lists of models                    |

### Data

- `0043_create_chatgpt_account.sql`, the provider module's version 3: `provider_chatgpt_account(id, host_id, client_id, subject, email, state, token_generation, token_parts, notice_seen, updated_at)`, with one row whose `id` is 1. The state is `signed_in` exactly when `token_parts` is above 0, `token_parts` is at most 32, and a client and a subject are set unless the state is `signed_out`. The row is created at the first attempt to sign in and never deleted, so the host ID and the client ID survive a sign-out.
- The credential store holds one bundle of tokens, `{"v":1,"access","refresh","expiresAt"}`, split into parts of at most 1,000 UTF-16 units under `provider/chatgpt/<generation>-<part>`. A write saves the new parts, then compares and sets `token_generation`, then deletes the old parts.
- The document of `provider_profiles` gains `access`, which an endpoint profile leaves out. A change of access counts as a change of the connection, so it raises the configuration revision, and recording the provider test's server keeps it.
- A plan profile's binding of provider-private data is `{profile}:{revision}:{account key}`, where the account key is the first 16 hex characters of SHA-256 over the client ID and the subject.

### Messages

The core's catalog gains the prefix `chat_chatgpt_`, for the account, the sign-in, the notice, the indicator and the plan's fields of the profile dialog, and `chat_failure_chatgpt_`, for the run failures. The browser's page after a sign-in takes its texts from the same catalog, since the core has none.

### Threat notes

- A local program that holds port 1455 or catches the redirect gets nothing it can use: Mannie fails with `port_in_use` before it opens anything, and PKCE with S256 and the `state` make a caught code useless (T-94).
- The tokens live only in the credential store, split into parts, each token is registered with the log redactor, and the webview never receives one (T-95).
- One refresh at a time, which holds the gate until the rotated tokens are stored, and the compare-and-set of generations keep two refreshes from racing the rotating refresh token (T-96).
- The plan's tokens never reach a worker or a plugin: the lender refuses plan profiles, and runs on them are offered no executors (T-97).
- The ID token is checked against OpenAI's keys with RS256 only, with its issuer, audience, nonce and expiry (T-98).
- The browser opens only on the owner's click, never for a run, and the authorize URL never reaches the webview (T-99).
- Script in the webview can start a sign-in, which still needs the owner in the browser, or sign out, which only ends the plan's use. It can choose a plan profile for a conversation as it can choose any profile ([security](../architecture/security.md#8-if-the-webview-is-compromised)).

### Platform differences

| Topic                | Windows                                                                                                              | Linux                                                                           |
| -------------------- | -------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| The credential store | Windows Credential Manager keeps at most 1,280 UTF-16 units in an entry, so the tokens are split into parts of 1,000 | The Secret Service has no such limit, and Mannie splits the tokens the same way |

## Acceptance

| Check                                                                                                                                                                                                                                                             | How it is tested                                                                                                                                     |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| Plan requests hold exactly their top-level keys, the namespace in the tools and in replayed calls, and the phase, and endpoint requests stay as they were                                                                                                         | The unit tests of `crates/mannie-provider/src/responses.rs`, and the conformance fixtures `open_ai/plan_*` with their `request.json`                 |
| A plan answer without a content type is read as its event stream, an answer of another type is refused, and other servers must send `text/event-stream`                                                                                                           | The tests of `crates/mannie-provider/tests/transport.rs`, and the conformance fixtures of the plan's streams, which have no content type             |
| Plan errors come before the classes of the HTTP status, a usage limit in a stream is permanent, the codes mean nothing on other servers, and `detail` bodies are read                                                                                             | The unit tests of `crates/mannie-provider/src/errors.rs`                                                                                             |
| The catalog keeps the listed models in the server's order, is read whole up to 16 MiB however it is cut into chunks, and is refused when it is not JSON or longer                                                                                                 | The unit tests of `crates/mannie-provider/src/detect.rs`, and the tests of the catalog in `crates/mannie-provider/tests/chatgpt.rs`                  |
| The authorize URLs of a registration and of a later sign-in, PKCE, and the codes of OpenAI's errors in both shapes                                                                                                                                                | The unit tests of `crates/mannie-provider/src/chatgpt/oauth.rs`                                                                                      |
| The code exchange, the refresh's form, the terminal codes in both shapes, transient failures, an answer without a new refresh token, a refused redirect and the revocation                                                                                        | `crates/mannie-provider/tests/chatgpt.rs`, against `FixtureServer`                                                                                   |
| ID tokens: a good one, a bad signature, a wrong `iss`, `aud` or `nonce`, an expired one, an unknown `kid`, and an `alg` other than RS256                                                                                                                          | The unit tests of `crates/mannie-provider/src/chatgpt/id_token.rs`, with tokens signed with the example RSA key of RFC 7515, appendix A.2            |
| The callback listener: a wrong state, an error, the timeout, a wrong path, an oversized request and a busy port                                                                                                                                                   | The unit tests of `crates/mannie-provider/src/chatgpt/callback.rs`                                                                                   |
| Migration 0043 from the previous release's database, the single account row, the compare-and-set of generations, and `access` through updates, `record_profile_server` and the revision                                                                           | The tests of `crates/mannie-store`                                                                                                                   |
| Refreshes: before expiry, one for concurrent runs and child agents, one that a stopped run does not lose, terminal and transient failures, the order of the writes, a locked vault, a sign-out that waits, every token in the redactor, and a shutdown that waits | `crates/mannie-runtime/tests/chatgpt.rs`                                                                                                             |
| Calls: a token for each call, one refresh and one repeat after a refusal, a run that a second refusal ends, no repeated tool operation, and summaries with fresh tokens that are cut down to their allowance                                                      | The same                                                                                                                                             |
| Accounts: a switch during a reply, a refresh that keeps the binding, the same account that keeps its reasoning, and another account that never receives it                                                                                                        | The same                                                                                                                                             |
| Workers: no executors offered, follow-ups refused, and lending refused                                                                                                                                                                                            | The same                                                                                                                                             |
| The provider test of a plan profile, a usage limit that is never retried, and unavailability retried within the policy                                                                                                                                            | The same                                                                                                                                             |
| A planted token reaches only the `Authorization` header, and never a log, the database file or IPC                                                                                                                                                                | The planted-secret tests                                                                                                                             |
| Sign in through the recorded link, the notice, a plan profile from the catalog, a reply with the indicator, a forced refresh, the usage limit with "Manage usage", and sign out                                                                                   | End to end, against the authorize, token, revocation and key endpoints, the catalog and the plan's answers of `tests/e2e/support/provider-server.ts` |
| The new views in WebView2's light and dark themes, from the keyboard, in the pseudo-locale, and with axe                                                                                                                                                          | The UI pass of M19                                                                                                                                   |
| Text, a tool call and its replay with the namespace, `phase` and encrypted reasoning, a refresh and a revocation, on the live route                                                                                                                               | `task provider:live:chatgpt`, whose fixtures replace the constructed ones ([M19 validation results](../roadmap/m19-validation.md))                   |
| A sign-in and the provider test of a plan profile, tool calls whose reasoning replays across turns, two child agents at once, a restart without a new sign-in, and a sign-out that revokes                                                                        | The owner's checks of the development build on Windows                                                                                               |
