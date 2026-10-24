# M13 validation results

|              |                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Draft for review                                                                                                                                                                                                                                                                                                                                                                                                   |
| Last updated | 2026-10-25                                                                                                                                                                                                                                                                                                                                                                                                         |
| Related      | [Milestones](milestones.md#m13-notebooks), [ADR-0045](../adr/0045-switch-the-sidebar-between-views-with-an-activity-bar.md), [ADR-0046](../adr/0046-let-plugins-attach-context-to-conversations.md), [ADR-0047](../adr/0047-serve-embeddings-and-reranking-through-provider-profiles.md), [ADR-0048](../adr/0048-erase-what-plugins-delete-from-the-store.md), [Notebooks](../../plugins/notebooks/docs/README.md) |

This document records what the spikes and checks of [M13](milestones.md#m13-notebooks) showed, so that the owner can review the plugin Notebooks and ADR-0045 to ADR-0048, and see what remains before the next release. The owner accepted ADR-0045 to ADR-0047 and the plugin's first ADR with the plan (D43). ADR-0048 is Proposed: building the plugin found that deleted text stayed in the store's write-ahead log, and the store service's erasing writes meet T-67. Each section names the checks, where they ran, the result, and what is open.

The checks ran on 2026-10-24 on the owner's Windows 11 workstation, with Rust 1.99.0, Node.js 24.15.0 and WebView2 155.0.4283.62 for the end-to-end tests.

## 1. Summary

| Spike or check                                                | Result                                                                                                     | Open                                                                                                      |
| ------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| S1: embeddings and rerank fixtures from the owner's servers   | Not run: no server of the owner was reachable from this session                                            | Live scenarios of search profiles in `task provider:live`, and fixtures recorded from the owner's servers |
| S2: pdf-extract on the owner's PDFs and on a malformed corpus | The malformed corpus fails without a crash, and 7 of the owner's PDFs read every page (section 7.1)        | The owner's other PDFs                                                                                    |
| S3: search by meaning and words against meaning alone         | A replay of the owner's conversation, with each message's predecessor, found what it missed (section 7.2)  | The owner's check in a notebook's chat                                                                    |
| The plugin's Rust tests                                       | Passed: 79 tests of the documents crate and the plugin, through the real host with a fake service `models` | None                                                                                                      |
| The plugin's UI tests                                         | Passed: 58 tests in 11 files                                                                               | None                                                                                                      |
| The plugin's end-to-end spec                                  | Passed its 7 tests on WebView2, with axe in both themes, after fixes of the faults that it found (7.3)     | The same spec on WebKitGTK                                                                                |
| `task ci`                                                     | 13 of 14 steps passed; `lint:deps` failed on three advisories (section 6)                                  | The owner's decision on ttf-parser, and updates of tokio and crypto-bigint                                |
| `task e2e`                                                    | 23 of 26 spec files passed; the specs of child agents and Workers failed on the report text of ffa11d4     | Those three tests, which expect the report text before ffa11d4                                            |

## 2. Embedding and rerank servers (S1)

Not run. The search adapters' tests answer with the shapes that llama.cpp's server, vLLM and Text Embeddings Inference document, constructed in `crates/mannie-provider/tests/search.rs`: an embedding answer out of order, Jina's shape of reranking sorted by score, TEI's bare array, logits that a sigmoid turns into probabilities, refusals of each error class, answers of another shape, and calls beyond the limits that never leave Mannie. `task provider:live` holds no scenario of search profiles yet, so recording needs live scenarios of their own, with a fixture of one request and one response. Until then, the test of a search profile in the Providers settings checks a server: it reports the dimensions of an embedding model and the scores of a reranker for a passage that answers a question and one that does not.

## 3. Extraction (S2)

`plugins/notebooks/crates/mannie-notebooks-documents/tests/pdf.rs` writes PDFs of its own and feeds the extractor a corpus of malformed files: an empty file, a file that is not a PDF, a header alone, a file cut in half, a cross-reference table past the end, 100,000 opening brackets, broken operators, an unknown font and pseudo-random bytes. Each one fails with `unreadable` or `no_text`, or reads, without a crash. A PDF that nests deeper than 256 levels outside its streams is refused before it is parsed, and one with more than 2,000 pages fails with `too_many_pages` before its text is read. Nesting inside a compressed stream stays bounded only by the extractor's stack of 256 MiB, which the plugin's ADR names as a residual risk. The owner's own PDFs have not been read yet.

## 4. Search (S3)

Not run. The plugin's tests check the parts of the search with constructed passages: BM25, the fusion by reciprocal rank, the cosine of normalized vectors, the choice of passages within the room and without overlap, and a conversation whose message gets the passage that applies and whose unrelated message gets the line that nothing applies. The threshold of 0.5 and the numbers of candidates wait for the owner's questions.

## 5. What the end-to-end spec found

The spec turns Notebooks on, adds search profiles for the mock provider's `/embeddings` and `/rerank` in the Providers settings, chooses them in the Notebooks settings, adds a Markdown file through the file dialog, starts a chat from the notebook, sends a message whose reply cites [1], opens the passage from the citation in the source's side panel, and turns Notebooks off. Its last step failed at first: turning the plugin of the selected entry of the activity bar off on the Plugins page closed the settings, because the bar's fallback to Chat navigated. On the settings page the bar now selects Chat without navigating, and the settings lead back to Chat.

## 6. The build's checks

`task ci` passed every step but `lint:deps`: formatting, the documents, the scripts' tests, the contracts, TOML, Clippy, ESLint, svelte-check, the catalog check, the desktop app's Vitest tests and the Rust tests of every workspace. Its first run found what this change broke, each fixed: a test of the worker that lacked the new field of instruction contexts, the release script's list of the plugins that ship, and the Workers link's snapshot of a conversation, which a summary's empty attachments changed until a summary without attachments left the field out. `lint:deps` reports three advisories:

| Advisory          | Crate                                                      | Cause                                                                       |
| ----------------- | ---------------------------------------------------------- | --------------------------------------------------------------------------- |
| RUSTSEC-2026-0386 | tokio 1.53.2, unsound, in every workspace                  | Before this change; tokio 1.53.3 fixes it                                   |
| RUSTSEC-2026-0387 | crypto-bigint 0.7.5, a vulnerability, in the SSH plugin    | Before this change                                                          |
| RUSTSEC-2026-0192 | ttf-parser 0.25.1, unmaintained, which lopdf 0.42 requires | This change: no feature of lopdf leaves it out, and no fixed version exists |

lopdf reads the fonts of PDFs with ttf-parser, inside the extractor's thread under the bounds of T-64. The repository accepts an unmaintained crate with a reason in `deny.toml`, which waits for the owner's decision.

`task e2e` ran the 26 spec files on WebView2. 23 passed, the plugin's included. The specs of child agents and Workers failed in three tests that expect a task's report as it read before ffa11d4, which the commit of M12 recorded. The chat spec failed first in two tests, whose helper counted the activity bar's link to Chat as a listed conversation; the helper now reads only the content sidebar, and the spec passes.

`task plugins:grants` lists Notebooks with the services `file-dialog` 1, `models` 1 (sensitive) and `store` 1, no secret entries, its 14 commands and the channel `follow_notebooks`, no programs, `pdf-extract` and `lopdf` as the crates that only it may use, and no native calls.

## 7. The second round (D44)

The owner tried the plugin on administration guides and asked for documents of any size, "@" in regular chats, chats of each notebook's own, the search of the earlier harness spatials, and the UI changes of notes 13 to 18 (D44). The checks below ran on 2026-10-25 on the same workstation.

### 7.1 The owner's PDFs (S2)

`BeyondTrust_Password_Safe_7.2_Admin_Guide.pdf` failed as unreadable. pdf-extract parses every image that a page draws as operators, an image on page 77 parses as `m` with one operand, pdf-extract panics, and the plugin's single `catch_unwind` failed the whole file. The extractor now empties image streams, reads each page under its own `catch_unwind`, and fails only when no page reads (plugin ADR-0002). A test PDF whose image breaks the parser fails without the fix and reads with it. All 7 of the owner's PDFs read every page, among them BeyondTrust (130 pages, 0.6 s) and Delinea Secret Server (1,546 pages, 27 s). The two Delinea files of 53 and 55 MB were refused only by the limit of 32 MiB, which is gone.

### 7.2 The search of the owner's conversation (S3)

In the owner's conversation about WALLIX, "does its connection work like this…" found a OneIdentity proxy page, "what does proves it in docs?" found a page about audit certificates, and "elaborate…" found nothing above the threshold of 0.5: each follow-up was searched with its own words alone. A replay of the same messages, each with the message before it, found the WALLIX SSH proxy passages, which the owner's reranker scored 0.97 to 0.998. The search now ranks each message alone and with the owner's two earlier messages, keeps no threshold and reranks 40 candidates instead of 12, and the model searches and reads the attached notebooks with tools (ADR-0049, ADR-0052). The owner's own check of the WALLIX conversation in a notebook's chat is open.

### 7.3 The build's checks

- The Rust tests: the documents crate's 29 tests, the plugin's 35 unit tests and 15 tests through the real host, among them a notebook's chat, the three tools and "@", and the runtime's 12 tests of attachments, among them a child whose tool calls carry its parent's notebooks. `task test` ran 1,408 Rust tests: 1,407 passed, and `mannie-secrets`'s test of a planted secret failed once under the full run's load. It passed in `task ci` and 6 times alone, and this round changes nothing that it reads.
- Vitest: 619 tests in 99 files, the plugin's 58 in 11 among them.
- `task ci` ran 14 steps: 9 passed. `fmt:check`, `lint:rust`, `lint:ts` and `lint:types` failed on files changed during the run and on 4 findings of Clippy, which were fixed, and each passes when run again. `lint:deps` fails on the three advisories of section 6.
- `task e2e` ran 26 spec files on WebView2, and 23 passed. The plugin's spec found two faults, both fixed: a notebook made from the empty view got its sidebar row before its page opened, so the row stayed closed and hid the notebook's chats, and the spec's selector of the Chat entry also matched the Notebooks entry, whose address was the chat view. Run again, the plugin's spec passes its 7 tests with axe in both themes. The specs of child agents and Workers fail in the three tests of section 6.

## 8. What remains open

- The owner's decision on ADR-0048, and on ttf-parser.
- S1, S2 and S3 with the owner's servers, PDFs and questions, and live scenarios of search profiles that record fixtures.
- The end-to-end tests on WebKitGTK.
- The owner's checks: search profiles for the servers that serve Qwen3-Embedding-0.6B and Qwen3-Reranker-0.6B pass their test, the two Delinea PDFs are added and ready, and the WALLIX conversation, repeated in a notebook's chat, cites the WALLIX SSH proxy passages.
