# Notebooks

|              |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Plugin       | `notebooks`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Milestone    | M13                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Last updated | 2026-10-25                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Related      | [ADR-0045](../../../docs/adr/0045-switch-the-sidebar-between-views-with-an-activity-bar.md), [ADR-0046](../../../docs/adr/0046-let-plugins-attach-context-to-conversations.md), [ADR-0047](../../../docs/adr/0047-serve-embeddings-and-reranking-through-provider-profiles.md), [ADR-0049](../../../docs/adr/0049-gather-context-for-the-messages-that-ask-and-cite-it-by-links.md), [ADR-0050](../../../docs/adr/0050-let-a-mention-attach-what-it-names.md), [ADR-0051](../../../docs/adr/0051-let-a-plugin-keep-conversations-of-its-own.md), [ADR-0052](../../../docs/adr/0052-offer-a-plugins-tools-where-its-attachments-are.md), [extract documents](adr/0001-extract-documents-with-pdf-extract-and-a-markdown-scanner.md), [documents of any size](adr/0002-read-documents-of-any-size.md), [Plugins](../../../docs/features/plugins.md), [Providers](../../../docs/features/chat/providers.md), [M13 validation results](../../../docs/roadmap/m13-validation.md) |

## Intent

The owner wants retrieval over documents in Mannie, after Google's NotebookLM: notebooks of documents that the owner adds, such as PDF and Markdown files, questions over them, and conversations that point at them, so that the passages that apply to a request reach the model. In the owner's words: "UI gets another sidebar, an icon only on the left, before the main sidebar. First item shows chat, second one the rag chat", "I can point chat session to those documents for additional context that may be applicable to a request", and "If I deploy those to some server I want to able to connect to that server with those two models. Nothing that's on my pc shall be hardcoded into Mannie" ([D43](../../../docs/roadmap/milestones.md#5-decisions)).

After trying the first version on administration guides, the owner asked for documents of any size, for "@" to attach notebooks in regular chats, for chats of each notebook's own, and for the search of the earlier harness spatials, whose answers were better: "Notebooks should have its own dedicated chat instead of reusing the standard one. They different in its purpose" ([D44](../../../docs/roadmap/milestones.md#5-decisions)).

## Behavior

### Turning it on

- Notebooks is a plugin on the Plugins page, with a switch. It starts off on a new installation, like every plugin ([D28](../../../docs/roadmap/milestones.md#5-decisions)).
- While Notebooks is off, the activity bar has no Notebooks entry. The notebooks' chats stay readable by their address, and conversations keep their attached notebooks, whose chips show their names. A reply in such a conversation gets no passages, and its owner's message shows that Notebooks is off. The notebooks, their sources and their passages stay.
- Turning Notebooks off while sources are indexed asks first, and their indexing continues when it is turned on again.

### The models

- Notebooks searches with an embedding model and, if the owner chooses one, a reranker. They run on servers that the owner names in the Providers settings as search profiles: the server's URL, the model's name, an optional key and, for an embedding model, the text that goes before each query and before each document ([ADR-0047](../../../docs/adr/0047-serve-embeddings-and-reranking-through-provider-profiles.md)). A model on this computer runs on a server here, such as llama.cpp's server.
- A new embedding profile starts with the query text that instruction-aware models such as Qwen3-Embedding want, "Instruct: Given a question, retrieve documentation passages that answer it" and a line "Query: ", which the owner can change or clear.
- The Notebooks settings pick the embedding profile, the rerank profile and the most passages that a message gets. Without an embedding profile, sources wait with "Choose an embedding model". The reranker only orders the passages, so without one a search keeps the order of its fused rankings.
- Changing the embedding profile, its model or its document text indexes every source again.

### Notebooks

- The activity bar's Notebooks entry shows the page of the most recently changed notebook. Without notebooks, it offers "New notebook".
- The sidebar lists the notebooks, newest first. Each opens its page, and its toggle shows "New chat" and the notebook's chats, the most recently changed first. A long title ends in "…".
- A notebook has a name, unique without regard to case. Its page shows its sources and "New chat", and the notebook's menu has "Rename…" and "Delete…".
- Deleting a notebook asks first, and says how many of its chats go with it and how many other chats use it. It deletes the notebook's chats, then its sources and their passages, and takes it from the other chats, which keep their messages. A chat that could not be deleted keeps the notebook, and the dialog says why.

### Sources

- "Add sources…" opens the system's file dialog for PDF, Markdown and text files. Files dropped on the notebook's sources are added too, and "Paste text…" adds a text that the owner writes or pastes, with a title. The files that were not added are listed with the reason of each until the owner dismisses them.
- A file of any size, a PDF of any number of pages and a pasted text of any length can be added, and a notebook holds any number of sources ([documents of any size](adr/0002-read-documents-of-any-size.md)). A file that the notebook already holds, byte for byte, is refused.
- A PDF is read page by page. A page that cannot be read is left out, and the PDF fails only when no page can be read.
- Each source shows its state: Reading, Waiting for an embedding model, Indexing with its progress, Paused with the reason while its server fails, Ready, or Failed with the reason, such as "This PDF has no text. It may be a scan." A source that failed while it was indexed has "Try again".
- Each source has a switch, "Use in answers", which is on when it is added. A source that is off is never searched or read by the model.
- Opening a source shows its text in a viewer that fills the window, page by page or section by section. "Close" and Escape close it. "Remove…" asks first and deletes the source's text and passages.

### Chats

- "New chat", on a notebook's page or under it in the sidebar, starts one of the notebook's chats ([ADR-0051](../../../docs/adr/0051-let-a-plugin-keep-conversations-of-its-own.md)). The chat belongs to the notebook: the notebook stays attached, and its chip has no button that detaches it. It never shows in the Chat list or in a project, and it opens in the main area while the sidebar keeps the notebooks. It has the usual tools.
- In a regular chat, "@" lists the notebooks whose names hold what the owner typed. A notebook chosen there is attached to the chat when the message is sent ([ADR-0050](../../../docs/adr/0050-let-a-mention-attach-what-it-names.md)). The chip of an attached notebook detaches it, and a conversation holds at most 8 attachments.

### Answers

- Every message of a notebook's chat, and a message that names a notebook with "@", searches the notebooks that it concerns before the model answers, with the message and the owner's message before it, and the reply shows "Searching notebooks…" meanwhile ([ADR-0049](../../../docs/adr/0049-gather-context-for-the-messages-that-ask-and-cite-it-by-links.md)). The best passages go to the model, 8 unless the settings say otherwise, each with its source, its place and its cite link. No score drops a passage.
- Other messages of a chat with notebooks attached search nothing by themselves. The model searches with three tools, which it and the child agents that work for the chat get while notebooks are attached: `list_notebook_sources`, `search_notebooks` and `read_notebook_source` ([ADR-0052](../../../docs/adr/0052-offer-a-plugins-tools-where-its-attachments-are.md)). Each call shows as a card, such as "Found 6 passages for "ssh proxy"".
- The model's instructions list the attached notebooks with their sources. They tell it to answer a notebook's chat from the notebook first, to search with one to three differently worded queries, to read around a passage, to cite every claim that it takes from a source by copying the source's cite link, and to say plainly what the documents do not cover.
- Below the owner's message, a row says what was found, such as "Searched Physics notes: 5 passages", and opens to the passages. A search that failed says why, such as that the embedding server could not be reached.
- The reply cites a passage by a link labeled with its source and its page or heading, such as "guide.pdf p. 12". The citation opens the passage as the model read it with a message, or says that the model read it with a tool, and "Open source" shows it in the viewer, scrolled to it and marked.

## Fit

- Builds on [ADR-0045](../../../docs/adr/0045-switch-the-sidebar-between-views-with-an-activity-bar.md): a navigation entry and a view, a sidebar panel of its entry, and the TypeScript SDK's views, conversations and parameters, whose `openView` can replace the address.
- Builds on [ADR-0046](../../../docs/adr/0046-let-plugins-attach-context-to-conversations.md) and [ADR-0049](../../../docs/adr/0049-gather-context-for-the-messages-that-ask-and-cite-it-by-links.md): a context provider whose keys are notebooks, with outlines and the owner's earlier messages, and a kind of context that opens cited passages.
- Builds on [ADR-0050](../../../docs/adr/0050-let-a-mention-attach-what-it-names.md): a mention provider with the context provider's ID, and a kind of mentions.
- Builds on [ADR-0051](../../../docs/adr/0051-let-a-plugin-keep-conversations-of-its-own.md): conversations whose home is a notebook, which the TypeScript SDK starts, lists and deletes.
- Builds on [ADR-0052](../../../docs/adr/0052-offer-a-plugins-tools-where-its-attachments-are.md): three tools offered where notebooks are attached, which read only the plugin's own data, and their tool kinds.
- Builds on [ADR-0047](../../../docs/adr/0047-serve-embeddings-and-reranking-through-provider-profiles.md): the service `models` with the owner's search profiles. The plugin reaches no server itself and has no `[native]` grant.
- Follows [Plugins](../../../docs/features/plugins.md): a switch, a start and a stop that take effect at once, the plugin's code in `plugins/notebooks/`, its UI files named `notebooks` and its messages under `notebooks_`.
- Must not change: the core's chat, tools and providers beyond the extension points above, and the other plugins.

## Open questions

None.

## Decisions

- 2026-10-24: the owner decided ([D43](../../../docs/roadmap/milestones.md#5-decisions)):
  - Notebooks is a plugin. The activity bar shows Chat first and Notebooks second, and "Find a command" and "Settings" stand at its foot.
  - The models run on servers that the owner names, wherever they are deployed, and nothing of the owner's machine is built into Mannie.
  - Approving the plan accepts ADR-0045, ADR-0046, ADR-0047 and the plugin's first ADR.
- 2026-10-24: after trying the first version, the owner decided ([D44](../../../docs/roadmap/milestones.md#5-decisions)), which changes three points of D43:
  - Documents have no hard limits, and a PDF page that fails costs that page.
  - A notebook has chats of its own, which belong to it and show only under it. A chat that only attached a notebook stays in the Chat list.
  - "@" attaches a notebook in a regular chat, instead of a button of the message box.
  - The search follows the earlier harness: no threshold, the owner's earlier messages in the query, passages cited by links, and tools that list, search and read the attached notebooks.
  - The UI changes of notes 13 to 18: the sidebar lists each notebook's chats, the list of notebooks goes, the viewer fills the window, the page shows only sources, the settings' fields take the column's width, and the button of the message box goes.
  - Approving the revised plan accepts ADR-0049, ADR-0050, ADR-0051, ADR-0052 and the plugin's second ADR.
- 2026-10-25: the defaults below were chosen after the earlier harness, and wait for the owner's review:
  - Passages have about 1,500 characters, with 200 shared with the next one.
  - A search takes the 50 best passages by meaning and the 50 best by words, for the message and for the earlier messages with it, fuses them, has the reranker order the 40 best, and sends at most 8, each cut to 2,000 characters.
  - The query adds the owner's 2 earlier messages, each cut to 4,000 characters.
  - A tool's search gives 6 passages unless the model asks for 1 to 12, and a read gives 6,000 characters unless it asks for 500 to 20,000.
  - The passages of one notebook stay in memory while it is searched, at most 256 MiB for all notebooks.
- 2026-10-24: decided while building, waiting for the owner's review:
  - A source whose server fails while it is indexed shows as paused, with the reason, until a batch is stored or the waits are used up.
  - Files dropped on the sources of a notebook's page are added, and the shell refuses files dropped elsewhere.
  - While a source is indexed, its passages are searched by their words, and by meaning once their vectors of the current model are stored.
  - Deleting a notebook or a source erases its text from every file of the store, which needs [ADR-0048](../../../docs/adr/0048-erase-what-plugins-delete-from-the-store.md).
  - The source viewer shows 64 pages or sections at a time.
- 2026-10-25: decided while building the second round, waiting for the owner's review:
  - A passage never spans two pages or sections, also when sections are short, so a cited passage always opens at one place of the viewer. The earlier harness joins short Markdown sections into one passage; PDFs, which the owner's guides are, are split the same way in both.
  - The view without a notebook replaces its address with the latest notebook's page, so that Back skips it.
  - A read by the model counts characters from the start of a section, and its result names the section and the character where to read on.
  - A notebook's chats are deleted before the notebook, so that a chat that could not be deleted never outlives its notebook.

## Parts

- [Summaries and notes](summaries-and-notes.md): a summary of each source, and the owner's notes in a notebook, which need plugins to reach chat models. An idea.
- [Scanned documents](scanned-documents.md): text recognition for scanned PDFs and images. An idea.
- [More formats](more-formats.md): Word, HTML, EPUB and other files. An idea.
- [Web sources](web-sources.md): pages that Mannie fetches by their address. An idea.
- [Mentions of sources](source-mentions.md): a source in the "@" list, whose whole text goes with one message. An idea.

## Design

### The plugin

`plugins/notebooks/crates/mannie-notebooks` defines the plugin `notebooks`:

- Its manifest, `plugins/notebooks/plugin.toml`, declares the services `store`, `file-dialog` and `models`, and the crates `pdf-extract` and `lopdf`, which only it may use. It has no `[native]` grant. Its default is off.
- Its start reads the notebooks and the preferences, contributes the context provider `notebooks`, the mention provider `notebooks` and the three tools, resumes the indexing of the sources that wait, and follows the changes of search profiles. Its stop ends the indexing, which a later start resumes.
- `plugins/notebooks/crates/mannie-notebooks-documents` extracts the text of PDF, Markdown and text files ([extract documents](adr/0001-extract-documents-with-pdf-extract-and-a-markdown-scanner.md), [documents of any size](adr/0002-read-documents-of-any-size.md)) and splits it into passages. It depends on no part of the SDK.
- One task, the indexer, owns the queue of sources to read and index, one source at a time, and reports its work to `Instance::work()`. Searches run beside it.

### Contracts

- Commands, which the host's dispatch answers with `plugin_off` while Notebooks is off ([ADR-0034](../../../docs/adr/0034-build-plugins-on-a-plugin-sdk.md), decision 8):
  - `create_notebook` takes a name and returns the notebook; `rename_notebook` and `delete_notebook` take its ID.
  - `pick_sources` takes a notebook, shows the file dialog and adds the chosen files, and returns what it added and what it refused, with a code for each file.
  - `start_upload` takes a notebook and a file's name and returns an upload's ID; `add_upload_part` takes the upload, the part's index and at most 4 MiB of its bytes in base64; `finish_upload` adds the file as the dialog does; `cancel_upload` drops it. A part out of order, or 5 minutes without a part, ends the upload.
  - `add_text_source` takes a notebook, a title and a text.
  - `remove_source`, `retry_source` and `set_source_included` take a source's ID.
  - `read_source` takes a source and the first section and returns at most 64 sections from there, with how many the source has; `locate_passage` takes a passage's key and returns its source, its section and its place in the section's text.
  - `read_preferences` and `save_preferences` read and write the profiles and the most passages of a message; `list_search_profiles` lists the embedding and the rerank profiles that the owner made.
  - The channel `follow_notebooks` sends every notebook with its sources and their states, then again after each change.
- Errors: `notebook_not_found`, `source_not_found`, `name_taken`, `refused` with the reason, `profile_not_found`, `passage_not_found`, `nothing_to_retry`, `dialog_failed`, `profiles_unavailable`, `upload_not_found`, `upload_part_out_of_order` and `storage_failed`. The refusals of a file are `duplicate`, `unsupported_type` and `unreadable`. A field that breaks a rule is the host's `invalid_input` with the field's name.
- A source fails with a code: `no_text`, `encrypted`, `unreadable` or `not_utf8` when its text cannot be read, `interrupted` when the plugin stopped while it was read, and `server_unreachable`, `server_error`, `input_too_long` or `storage_failed` when its indexing failed. A source that is indexed while its server fails keeps its state with the code of the failure, and shows as paused.
- The constants that the UI reads are the limits of names and titles, the bytes of an upload's part, the sections of one read and the range of the passages of a message.
- The context provider `notebooks` takes notebook IDs as keys, describes a key by the notebook's name, refuses an unknown one with `notebook_not_found`, and gathers passages whose keys are `<source ID>:<passage number>`. Its outline names each notebook with its ID and up to 80 of its sources that answers use, with their IDs, kinds, pages and states. Its failure codes are `notebook_not_found`, `no_embedding_profile`, `nothing_indexed`, `storage_failed`, `server_unreachable`, `server_error` and `input_too_long`.
- The mention provider `notebooks` lists the notebooks whose names hold the query, those that start with it first, then the most recently changed, with the variant `attachment`. The core attaches a mentioned notebook through the context provider and never resolves the mention.
- The tools, offered to the agents `conversation`, `general` and `explore` where notebooks are attached, read only the plugin's data and reach only the notebooks that the call's attachments name and their sources that answers use; any other refuses the call before it runs:
  - `list_notebook_sources(notebook)` lists the sources of one attached notebook, or of all of them.
  - `search_notebooks(query, notebooks, sources, limit)` searches as a message does, without earlier messages, and gives each passage with its source, place, passage number, cite link and up to 2,000 characters. It fails with `no_embedding_profile`, `nothing_indexed`, `storage_failed` or a code of the embedding server, and when the server fails it searches by words and says so.
  - `read_notebook_source(source, passage, page, page_end, section, offset, limit)` reads around a passage, from a page through another, or from a character of a section. Each page or section starts with a line that holds its cite link, a read from the start of a Markdown source begins with its sections, and the result names where to read on. It fails with `page_not_found`, `passage_not_found`, `section_not_found`, `no_text` or `storage_failed`.
  - The data of a call's card holds the query and the number of passages found, or the source's name with the pages or the heading that it read.

### Data

- The plugin's migration 1 creates:
  - `notebooks_notebooks`: the ID, the name, unique without regard to case, and the times of creation and change.
  - `notebooks_sources`: the ID, the notebook, the kind (`pdf`, `markdown` or `text`), the name, the size, the SHA-256 of the bytes, unique within the notebook, the number of pages, the state (`reading`, `waiting`, `indexing`, `ready` or `failed`), the failure code, the switch, the counts of passages and of embedded passages, and the times of adding and indexing.
  - `notebooks_sections`: the text of each page or section of a source, with its place as a JSON document: a page number, a path of headings, or the whole text.
  - `notebooks_passages`: the passages of a source, in order, with their section, their text and the start and end of it in the section's text.
  - `notebooks_vectors`: a passage's vector as little-endian 32-bit floats, normalized, with the key of the embedding profile's model that made it.
  - `notebooks_preferences`: one row with the embedding and rerank profiles and the most passages of a message. A document of the first version also holds the reranker's least score, which no longer applies.
- Migration 2 adds `notebooks_sources.chunker`, the version of the splitter that made a source's passages.
- Deleting a notebook deletes its sources, and deleting a source its sections, passages and vectors, in one transaction that erases them from every file of the store ([ADR-0048](../../../docs/adr/0048-erase-what-plugins-delete-from-the-store.md), proposed).

### Indexing

- Adding a file reads it once through the dialog's handle, or takes the uploaded bytes, checks its type and its hash, and stores the source as `reading`. The indexer extracts its text, stores the sections, and splits them into passages: about 1,500 characters, split at the end of a paragraph, then a line, then a sentence, then a word, with 200 characters shared with the next passage, never across sections.
- A source whose passages an earlier splitter made is split again from its stored sections when the plugin starts, without reading its file, and indexed again.
- The indexer embeds each passage as a document with a header of its source's name and its place, 32 passages to a call, stores the vectors, and counts the progress. A server that cannot be reached, or that fails, pauses the source, which shows the reason, waits and tries again after 1, 2, 4, 8 and 15 minutes, and then fails with the reason until the owner tries again. A batch that is stored clears the reason.
- A source that was being read when the plugin stopped fails with `interrupted`, since its bytes are gone; one that was being indexed resumes from its stored passages.
- The embedding profile's model key names what a vector means. When it changes, every source is indexed again, and until a passage has a vector of the new model, it is searched by its words only.

### Search

- A search ranks the passages of the notebooks' sources that are on and ready or being indexed, for the query, at most its last 4,000 characters, and, when the owner wrote earlier messages, for those messages and the query together:
  - by meaning: the cosine of each query's vector and the vector of each passage that the current model made, the best 50 of each, with both queries embedded in one call;
  - by words: BM25 with k1 = 1.2 and b = 0.75 over the passages' words, the best 50 for each query. Words are runs of letters and digits in lower case, without 61 common English words, with a plural's "s" and "ies" taken off, identifiers split at their underscores and capitals besides the whole, and each pair of neighboring Han, kana or Hangul characters a word;
  - all rankings fused by reciprocal rank with k = 60.
- The rerank profile scores the 40 best of the fused list against the earlier messages and the query, and orders them by their scores, the rest after them. A reranker that fails leaves the fused order. No score drops a passage.
- An embedding server that fails, other than a profile that is gone, leaves the search to the words, and the log says so.
- A message's search sends the best passages that fit: at most the number that the settings say, 8 by default, and fewer when they would pass the room that the core gives. A passage that shares more than half of its text with one that goes is left out.
- The vectors and the words of a notebook's passages stay in memory after its first search, at most 256 MiB for all notebooks, and the notebook used least recently leaves first. Adding, removing and switching sources update them.

### The text the model sees

| Case                      | What the model reads                                                                                                                                              |
| ------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Passages                  | The core's block of excerpts of the owner's documents, each as "[n] {source}, {page or headings} · cite: [{source} p. {page}](cite:notebooks/{key})" and its text |
| A notebook named with "@" | "[The owner attached "{name}" to this conversation with this message.]" before the excerpts                                                                       |
| A search with no passage  | "[The attached {names} hold nothing that applies to this message.]"                                                                                               |
| A search that failed      | "[Mannie could not search the attached {names}: {code}.]"                                                                                                         |
| Notebooks off             | "[The attached {names} were not searched, because their plugin is off.]"                                                                                          |
| The instructions          | The guidance of ADR-0049, decision 6, then "The attached sources:" and the outline of each notebook                                                               |
| A tool's result           | The lines of `search_notebooks`, `read_notebook_source` and `list_notebook_sources` above, with cite links                                                        |

### Interface

- The view `notebooks` shows, with the parameter `notebook`, one notebook's page, with its sources, whose section takes dropped files, and "New chat". Without the parameter, it replaces its address with the page of the most recently changed notebook, or offers "New notebook". The shell's heading names the notebook.
- The dialogs, to name, rename and delete a notebook, paste a text, remove a source and view a source, are the overlay `notebooks.dialogs`, so the view, the sidebar, the command palette's "New notebook…" and the citations of the transcript open them alike.
- The viewer of a source is a dialog that fills the window, with "Close" and Escape. It shows 64 pages or sections at a time with "Earlier" and "Later", and scrolls to the passage of a citation, which it marks. It gives the focus back to the element that opened it.
- The sidebar panel `notebooks.notebooks` belongs to the navigation entry `notebooks` and lists the notebooks, each of which opens its page, with "New chat" and the notebook's chats in a collapsible group. A row opens when its notebook's page opens or a chat of it starts, and keeps its state while other views show.
- "New chat" starts a conversation through the TypeScript SDK with the notebook as its home, and deleting a notebook deletes its chats through the SDK's `deleteConversation`, then detaches it from the other chats.
- The context kind `notebooks` gives the book icon, the words of the plugin's failure codes, and opens a passage's key in the viewer.
- The mention kind `notebooks` words the group "Notebooks", the variant `attachment` and the summary `attached`.
- The tool kinds of the three tools give their icons, their lines and the words of their failure codes.
- The settings section Notebooks picks the profiles, with "None" for each and a way to the Providers settings when there is no embedding profile, and sets the most passages of a message, which "Save" keeps. Its fields take the column's width.
- Messages live in `plugins/notebooks/messages/en.json` under `notebooks_`.

### Loading, empty and error states

| View                      | Loading                       | Empty                                                         | Error                                                                                                       |
| ------------------------- | ----------------------------- | ------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| The view without notebook | Placeholders of two notebooks | "No notebooks yet", with "New notebook"                       | "The notebooks could not be loaded" with "Try again"                                                        |
| A notebook's page         | Placeholders of three sources | "No sources yet", with "Add sources…" and the drop zone       | "The notebook could not be loaded" with "Try again"; a deleted one says so, with "Open the latest notebook" |
| The sidebar panel         | Placeholders of two rows      | "No notebooks yet"                                            | "The notebooks could not be loaded"                                                                         |
| The source viewer         | Placeholders of three lines   | "This source has no text."                                    | "The source could not be loaded" with "Try again"; a removed source says so, with "Close"                   |
| The settings section      | Placeholders of two fields    | "No embedding models yet", with "Open the Providers settings" | "The settings could not be loaded" with "Try again"                                                         |

### Threat notes

- A hostile document can try to exhaust the extractor, and no size or page limit bounds it any more. Each page is read on its own under `catch_unwind`, image streams are emptied before any page is read, extraction runs on a thread of its own with a large stack, and a panic fails only its page. A PDF that nests deeper than 256 levels outside its streams is refused before it is parsed, since an overflowing stack ends the process; nesting inside a compressed stream is bounded only by the stack (T-64).
- A document can hold text that tries to instruct the model. Its passages reach the model as excerpts of the owner's documents and as tools' results, to use as reference and never as instructions, and content grants nothing (T-65).
- The text of documents and the owner's messages, the earlier ones too, go to the servers of the search profiles that the owner made, as messages go to chat providers, with the same credential and transport rules (T-66).
- The tools reach only the notebooks that the core names as the conversation's attachments, and the plugin checks every source and notebook that the model names against them. A child agent reaches the notebooks of the conversation that gave it its task.
- A cite link's key comes from the model, so "Open source" checks it with `locate_passage`, which finds nothing for a key of another source or shape.
- A compromised webview can add a source of any bytes, which the extractor treats as hostile anyway, can read every notebook, which it could already read through the conversations that use them, and can delete a conversation, which it could already do through the core's command.
- Deleted text leaves every file of the store: `secure_delete` overwrites the freed pages, and the deleting write empties the write-ahead log ([ADR-0048](../../../docs/adr/0048-erase-what-plugins-delete-from-the-store.md)). A planted text is searched for in the store's files after its source is removed (T-67).
- Logs at `info` name notebooks and sources by ID, and searches by their counts and times, never by their text.

### Platform differences

None. The file dialog is the system's on each platform, through the core.

## Acceptance

- The scanner of Markdown, the extraction of PDFs from fixture files and a corpus of malformed ones, a page that panics, an image that breaks the parser, a PDF of 2,101 pages, the splitting into passages and its version, BM25 and its words, the fusion, the cosine, the choice of passages, uploads, the mention provider's order and the tools' reads pass their unit tests in `plugins/notebooks/crates/`.
- Through the real host (`plugins/notebooks/crates/mannie-notebooks/tests/plugin.rs`), with a fake service `models`: notebooks and sources keep their rules, a duplicate file is refused, an upload takes its parts in order and ends when it is cancelled, a source is read, split, embedded and searched, a server that is down pauses and resumes it, a change of model indexes again, removing a source leaves none of its planted text, a notebook's chat searches each message with the message before it and outlines the notebook in the instructions, the tools list, search and read only the attached notebooks, "@" attaches a notebook whose tools later messages get, and the plugin passes the SDK's conformance check.
- The core's runtime tests: a notebook's chat gathers on every message, a regular chat only for a message that names a notebook, the earlier messages reach the provider, the home stays attached and keeps its chat out of projects, tools offered where attachments are get their keys, a child gets its parent's.
- The plugin's UI passes its Vitest tests in `plugins/notebooks/tests/ui/`, and the core's in `tests/apps/desktop/`: cite links become citations, the Chat list leaves notebooks' chats out, the home's chip has no detach button, the draft holds its home.
- In the app (`plugins/notebooks/tests/e2e/notebooks.test.ts`), with axe in both themes and the e2e provider's embedding and rerank endpoints: the activity bar opens the latest notebook; a notebook takes a Markdown file and shows it ready; a chat of the notebook shows under it and not in the Chat list, and sends a message whose reply cites a passage by a link, which opens the viewer at the passage; "@" in a regular chat attaches a notebook; and turning Notebooks off leaves the chat's parts readable.
- The owner's checks: search profiles for the servers that serve Qwen3-Embedding-0.6B and Qwen3-Reranker-0.6B pass their provider test, the two Delinea PDFs and the BeyondTrust guide are added and ready, and the WALLIX conversation, repeated in a notebook's chat, cites the WALLIX SSH proxy passages.
