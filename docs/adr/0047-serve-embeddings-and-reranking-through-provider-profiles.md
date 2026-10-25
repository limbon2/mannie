# ADR-0047: Serve embeddings and reranking through provider profiles, and let plugins use them

|          |                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D43)                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Date     | 2026-10-24                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Related  | [ADR-0010](0010-keep-provider-credentials-on-the-master.md), [ADR-0011](0011-own-the-provider-adapters.md), [ADR-0034](0034-build-plugins-on-a-plugin-sdk.md), [ADR-0043](0043-let-plugins-offer-the-tools-of-tool-servers.md), [ADR-0046](0046-let-plugins-attach-context-to-conversations.md), [Providers](../features/chat/providers.md), [Security](../architecture/security.md), [Notebooks](../../plugins/notebooks/docs/README.md) |

## Context

- The plugin Notebooks searches the owner's documents with an embedding model and a reranker. The owner wants them served by servers of the owner's choice, on this computer or elsewhere, and nothing of the owner's machine built into Mannie: "If I deploy those to some server I want to able to connect to that server with those two models" ([decision D43](../roadmap/milestones.md#5-decisions)).
- Providers today ([providers](../features/chat/providers.md)):
  - A profile names one chat model on one server, which speaks the Responses API or Chat Completions.
  - Its credential lives in the operating system's credential store ([ADR-0010](0010-keep-provider-credentials-on-the-master.md)) and travels over plain HTTP only to a loopback or private address, and redirects are refused.
  - The provider test checks a profile, and recorded fixtures back every adapter and quirk (V8).
  - A chat profile's contract and row carry fields that mean nothing to an embedding or rerank model: the context window, the output limit, the reasoning effort and the use of tools.
- Nothing in Mannie embeds or reranks text, and plugins reach no model: the SDK has no network service, and the plugin MCP opens its own sockets for its servers ([ADR-0043](0043-let-plugins-offer-the-tools-of-tool-servers.md), decision 8).
- Facts verified on 2026-10-24:
  - llama.cpp's server answers `POST /v1/embeddings` in OpenAI's shape. Its `POST /v1/rerank`, an alias of `/reranking`, takes `query` and `documents` and answers in Jina's shape, `results` of `index` and `relevance_score`, sorted by score, with scores as probabilities from 0 to 1; with `texts` in place of `documents`, it answers in the shape of Hugging Face's Text Embeddings Inference, an array of `index` and `score`. It serves them with `--embedding` and a pooling, or with `--reranking`.
  - vLLM serves `/v1/embeddings` in OpenAI's shape, and `/rerank`, `/v1/rerank` and `/v2/rerank` after Jina's v1 rerank API and Cohere's v1 and v2.
  - Cross-encoders and Qwen3's rerankers score with an unbounded logit, which some servers return as it is and others turn into a probability with a sigmoid.
  - Instruction-aware embedding models, such as Qwen3-Embedding, want an instruction before each query and nothing before documents, and E5 models want a prefix before both.

## Decision

1. The provider module gets search profiles. A search profile names one embedding or rerank model on one server:
   - a name, unique among search profiles regardless of case, and a kind, `embeddings` or `rerank`;
   - a base URL under the rules of chat profiles, a model, and an optional credential;
   - for an embedding profile, an optional query prefix and an optional document prefix of at most 1,000 characters each, which Mannie puts before each query or each document that it embeds.

   No model, URL or prefix is built into Mannie. Search profiles live in a new table, `provider_search_profiles`, with a configuration revision, and their credentials in the credential store as `provider/<profile ID>/<revision>`, under the rules of chat profiles. They never show among chat profiles, and the chat profiles' contracts do not change. Their commands are `list_search_profiles`, `create_search_profile`, `update_search_profile`, `delete_search_profile` and `test_search_profile`, and the Providers settings page lists them below the chat profiles.

2. Two adapters of `mannie-provider` call them, each with one request and one response and no stream:
   - embeddings as `POST {base}/embeddings` with the model, `input` as a list and `encoding_format: "float"`, reading `data` by its `index`;
   - reranking as `POST {base}/rerank` with the model, `query`, `documents` and `top_n`, reading `results` by their `index`.

   They share the chat adapters' transport, error classes, retries of transient errors and refusal of redirects, with limits of their own: 15 seconds to connect, 120 seconds for the response and at most 16 MiB of it. The rerank adapter reads the `results` of Jina and Cohere and the bare array of Text Embeddings Inference, and reads a list of scores with one outside 0 to 1 as logits, which a sigmoid turns into probabilities. Quirk profiles that recorded fixtures show switch other documented normalizations on, such as a server's largest batch.

3. The provider test of a search profile reads its credential, lists the server's models where it offers `/models` and checks that the model is among them, then embeds a short text and reports the vectors' dimensions, or reranks one passage that answers a question and one that does not, and reports both scores and whether their order is right.
4. A new service of the SDK, `models` (major 1, sensitive, since it sends the owner's text to a server):
   - `search_profiles(kind)` lists the search profiles of a kind by ID, name and model, without their URLs or credentials;
   - `embed(profile, input kind, inputs)` embeds at most 64 inputs of at most 32 KiB each, as queries or as documents, and returns one vector for each, all of one dimension;
   - `rerank(profile, query, documents)` scores at most 64 documents of at most 32 KiB each and returns one score for each, in their order, as a probability;
   - `changes()` streams the IDs of the search profiles that changed or were deleted.

   The plugin never sees a credential, a URL or a prefix. Its calls are logged by their outcome and size, without their text.

5. Fixtures are recorded from the owner's servers with `task provider:record`, together with constructed cases of errors, and the conformance tests run against them. Mannie reaches only the servers of the search profiles that the owner made (NFR-OPS-04).

Earlier decisions that this ADR changes:

| ADR  | Decision           | Change                                                                                                                                                                                         |
| ---- | ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 0010 | d1, d3, d4         | Refined: the credentials of search profiles are stored, referenced and held as the credentials of chat profiles                                                                                |
| 0011 | d1, d2, d3, d5, d7 | Refined: two adapters call embedding and rerank models with one request and one response, share the transport, the error classes and the quirk profiles, and are tested with recorded fixtures |
| 0034 | d5                 | Refined: the service `models`                                                                                                                                                                  |

## Consequences

- Positive: the owner runs embedding and rerank models where the hardware is, on a GPU server or this computer, and points Mannie at them as at a chat model. Credentials, transport rules, tests and fixtures stay in one place, and a later feature of the core or a plugin can embed and rerank through the same service.
- Negative: the owner runs a server for each model, also on this computer. The provider module gets a second kind of profile with its own table, commands and settings list. A search waits for two calls to a server.
- Risks: servers differ in the shapes and scales of their answers more than the recorded quirks cover; the provider test shows the scores of a known pair, so the owner sees a scale that is off, and a new fixture adds the quirk. Documents and messages go to the servers that the owner chose, as messages go to chat providers.

## Alternatives considered

| Option                                                | Why not chosen                                                                                                       |
| ----------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| Run the models inside Mannie, from their files        | The owner wants to connect to the servers where the models are deployed, and nothing of the owner's machine built in |
| Embedding and rerank interfaces on chat profiles      | Their contracts and rows carry chat's fields, and every chat profile list and picker would have to leave these out   |
| The plugin's own connections under `[native] sockets` | A second copy of the transport, the credential rules, the error classes, the test and the fixtures, in one plugin    |
| OpenAI's shape for reranking                          | OpenAI defines no rerank API, and the servers that rerank speak Jina's and Cohere's shape                            |

## Validation

M13: the adapters' conformance tests with recorded and constructed fixtures, the quirks among them; the rules and storage tests of search profiles, with a planted credential that no log or table holds; the service's tests of its limits; and the owner's provider tests against the servers that serve the first two models.
