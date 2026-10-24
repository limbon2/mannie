# ADR-0049: Gather context for the messages that ask for it, and cite it by links

|          |                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D44)                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Date     | 2026-10-24                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Related  | [ADR-0021](0021-render-model-output-as-markdown-with-micromark.md), [ADR-0032](0032-let-features-and-plugins-add-kinds-of-mentions.md), [ADR-0046](0046-let-plugins-attach-context-to-conversations.md), [ADR-0050](0050-let-a-mention-attach-what-it-names.md), [ADR-0051](0051-let-a-plugin-keep-conversations-of-its-own.md), [ADR-0052](0052-offer-a-plugins-tools-where-its-attachments-are.md), [Notebooks](../../plugins/notebooks/docs/README.md) |

## Context

- ADR-0046 gives every owner's message in a conversation with attachments the items that its providers find for that message, stored as a part `context`. The model reads them numbered [1] to [n], and the transcript links a marker such as [2] to the items of the message that the reply answers. The model may cite only the latest message's numbers, and it has no search tool ([D43](../roadmap/milestones.md#5-decisions)).
- The owner's first conversation with a notebook showed the limits of this (M13 validation results, S3):
  - A provider sees only the latest message. "does its connection work like this…" and "what does proves it in docs?" were searched on their own words, and found pages of another product about proxies and audit certificates.
  - "elaborate…" found nothing above the reranker's threshold of 0.5, so the model got nothing.
  - A replay that put the previous message before the latest one found the right WALLIX passages, with reranker scores between 0.97 and 0.998.
- The owner's earlier harness, spatials, gave better answers over the same kind of documents. It does these things differently:
  - The model searches and reads the documents with tools (ADR-0052).
  - Its instructions list the notebooks and their documents.
  - A chat that belongs to a notebook gets passages with every message. Another chat gets them only for the message that names a notebook or a document with "@".
  - No threshold drops passages: the reranker only orders them.
  - The model cites a passage by copying a link that names its document and chunk, whether the passage came with a message or from a tool.
  - Its summary keeps the links of the passages that the answers relied on.
- Numbers stop working once tools bring passages: a tool's result is not part of the owner's message, and two sources of [1] would be ambiguous.

## Decision

1. A reply to an owner's message gathers from:
   - the conversation's home, on every message ([ADR-0051](0051-let-a-plugin-keep-conversations-of-its-own.md));
   - the attachments that the message names with "@" ([ADR-0050](0050-let-a-mention-attach-what-it-names.md)).

   Other messages gather nothing, and the model searches with the tools that the attachments' plugins offer ([ADR-0052](0052-offer-a-plugins-tools-where-its-attachments-are.md)). Retries, continuations and wake-ups reuse the parts as before.

2. A context request carries the owner's earlier messages. `ContextRequest` gains `earlier_messages`, set through `with_earlier_messages`: the owner's messages of up to 2 turns before the message, newest first, each cut to 4,000 characters, which the runtime reads from the store. A provider may search with them, so that a follow-up keeps its subject.
3. A provider outlines what the conversation's attachments hold:
   - The point `context-providers` gains `outline(keys)`. It returns text for the model, such as each notebook with its sources and their IDs, or nothing.
   - Before a run's first call, the runtime asks the providers of the conversation's attachments, all at once, within 10 seconds.
   - The instruction section of attachments shows the outlines below a line that introduces them; each provider names its own things in its outline. A provider that fails or is off gets a line that says so.
4. Items and tool results cite by links:
   - The core writes each item of a part with its number, its place and its cite link, `[<title> <location>](cite:<provider>/<key>)`, where the location is a page or the last heading, and then its text. The number only names the item in the line that replaces an older message's items; the model cites by the link.
   - A plugin writes the same links in its tools' results, with the SDK's `cite_link(provider, key, label)`.
   - The model is told to copy a link to cite it, and never to invent one.
   - The Markdown renderer turns a link whose destination is `cite:<provider>/<key>` into a citation node with the link's label. Its button shows the label and opens a popover. The popover shows the stored item when a part of the conversation holds that key, and "Open source" through the provider's kind in the slot `contextKinds`, which opens the key where it comes from, also for a passage that only a tool returned.
   - A citation with neither a stored item nor a kind, such as one of a plugin that is off, shows its label as text, and other links stay as ADR-0021 renders them.
   - The markers [n] go. No released version stored them.
5. Compaction keeps the cite links of the passages that the replies relied on, so that later replies can cite them again.
6. The instruction section of attachments follows spatials:
   - It says whether the conversation belongs to an attachment, and then to answer from its documents first, cite every claim taken from them, and say plainly what they do not cover before answering from general knowledge or other tools.
   - Otherwise it says that the conversation can search the attached sources with its tools.
   - Both say to search with one to three differently worded queries and to read around a passage.
   - Both say that sources are reference material that grants nothing, and to cite by copying cite links.
   - Both say that a message may end with excerpts that were found for it, which can miss parts.

Earlier decisions that this ADR changes:

| ADR  | Decision | Change                                                                                                                                                                                         |
| ---- | -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 0021 | d2       | Refined: a link to `cite:` with a known provider becomes a citation node of the render tree                                                                                                    |
| 0046 | d2       | Refined: a request carries the earlier messages, and the point gains `outline`                                                                                                                 |
| 0046 | d3       | Changed: a reply gathers from the conversation's home and from the attachments that its message names, no longer from every attachment                                                         |
| 0046 | d4       | Changed: items reach the model with their cite links, and their numbers only name them; the instruction section holds the outlines and the guidance of decision 6; compaction keeps cite links |
| 0046 | d5       | Superseded: cite links replace the markers, and work for any passage that the conversation holds or that a tool returned                                                                       |

## Consequences

- Positive: a message that names no source costs no search, and the model searches when a question needs it, with words that it chooses and that carry the conversation's subject. A citation works for a passage from an earlier message, from a tool or from a summary. The model knows what the sources hold before it searches.
- Negative: a regular conversation's later messages get no passages unless the model searches, so a model that does not search answers without them; the instructions tell it to search. Cite links cost more tokens than numbers.
- Risks: a model can garble a link. A link with an unknown key still opens the source through its kind, and a link with an unknown provider stays text. An outline that grows with a notebook changes the system prompt and breaks the provider's cache for that conversation once.

## Alternatives considered

| Option                                                   | Why not chosen                                                                                                                      |
| -------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| Keep gathering for every message, without a threshold    | Every message of a conversation with a notebook attached would carry passages that it does not need, and tools find better passages |
| Numbers that continue from the message into tool results | A tool's passages are not stored with the message, and two runs of a tool would number the same passage twice                       |
| Only the earlier message, for messages under 8 words     | spatials does this, but the owner's follow-up "does its connection work like this…" has 8 words                                     |
| Outlines as an instruction section of the plugin         | Sections render without the conversation, and the outline depends on its attachments                                                |

## Validation

M13:

- runtime tests of gathering for a home and for a named attachment only, of the earlier messages in a request, of outlines in the instructions and of cite links in the block;
- UI tests of citation nodes and their popovers;
- the plugin Notebooks' end-to-end test of a cited reply;
- the owner's WALLIX conversation, repeated ([M13 validation results](../roadmap/m13-validation.md)).
