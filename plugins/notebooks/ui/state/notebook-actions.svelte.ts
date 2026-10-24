// What the owner does with notebooks and their sources (plugins/notebooks/
// docs/README.md, notebooks and sources): creating, renaming and deleting a
// notebook, adding files through the system's dialog or a drop and pasted
// texts, switching, trying again and removing sources, and starting one of a
// notebook's own chats. The dialogs are mounted once, as the plugin's
// overlay, so that the sidebar, the view and the command palette open them
// alike. The plugin's commands go through its generated bindings (ADR-0034,
// decision 9), and the conversations and their attachments through the
// core's commands (ADR-0046, decision 6; ADR-0051).
import { conversationFailureMessage, deleteConversation, detach } from '@mannie/plugin-sdk/conversations';
import { call } from '@mannie/plugin-sdk/ipc';
import { startConversation } from '@mannie/plugin-sdk/navigation';

import { commands, type NotebookView, type SourceRefusal, type SourceView } from '../generated/bindings.js';
import { m } from '../generated/messages.js';
import { PLUGIN_ID, UPLOAD_PART_BYTES } from '../generated/plugin.js';
import type { FileRefusal, NotebooksFailure } from '../types/notebooks.js';
import { chatsOf, chatsUsing } from '../utils/notebooks-chats.js';
import { NOTEBOOKS_CONTEXT } from '../utils/notebooks-context.js';
import { readAsBase64, takesDocument } from '../utils/notebooks-files.js';
import { failureCode } from '../utils/notebooks-messages.js';
import { openNotebook, openNotebooks, shownNotebookId } from '../utils/notebooks-places.js';

import { notebookLibrary } from './notebooks.svelte.js';
import { panelRows } from './panel-rows.svelte.js';

/** One opening of the name dialog, for a new notebook or one to rename. */
export class NameRequest {
  /** Why the latest attempt failed. */
  failure: NotebooksFailure | null = $state(null);
  name = $state('');
  /** The notebook to rename, or `null` for a new one. */
  readonly notebook: NotebookView | null;
  saving = $state(false);

  constructor(notebook: NotebookView | null) {
    this.notebook = notebook;
    this.name = notebook?.name ?? '';
  }
}

/** One opening of the deletion confirmation. */
export class DeleteRequest {
  /** Why one of the notebook's own chats was not deleted, which keeps the notebook. */
  chatFailure: string | null = $state(null);
  deleting = $state(false);
  /** Why another chat still has the notebook attached after it was deleted. */
  detachFailure: string | null = $state(null);
  failure: NotebooksFailure | null = $state(null);
  readonly notebook: NotebookView;
  /** The notebook's own chats when the confirmation opened, which go with it. */
  readonly ownChats: number;
  /** The other chats that attached the notebook when the confirmation opened, which keep their messages. */
  readonly usingChats: number;

  constructor(notebook: NotebookView, ownChats: number, usingChats: number) {
    this.notebook = notebook;
    this.ownChats = ownChats;
    this.usingChats = usingChats;
  }
}

/** One opening of "Paste text". */
export class PasteRequest {
  failure: NotebooksFailure | null = $state(null);
  /** The notebook that takes the text. */
  readonly notebook: NotebookView;
  saving = $state(false);
  text = $state('');
  title = $state('');

  constructor(notebook: NotebookView) {
    this.notebook = notebook;
  }
}

/** One opening of the removal confirmation of a source. */
export class RemoveRequest {
  failure: NotebooksFailure | null = $state(null);
  removing = $state(false);
  readonly source: SourceView;

  constructor(source: SourceView) {
    this.source = source;
  }
}

/** The plugin's refusal of a file, from a failed call, or `null` for a failure of another kind. */
function refusalOf(failure: NotebooksFailure): SourceRefusal | null {
  if (failure.kind === 'ipc') {
    return null;
  }
  const { error } = failure;
  const isRefusal = error.origin === 'plugin' && error.code === 'refused';
  return isRefusal ? error.reason : null;
}

/** The owner's actions on notebooks and sources, and the dialogs they open. */
export class NotebookActions {
  /** Why adding files to a notebook failed as a whole, such as its dialog, by the notebook's ID. */
  addFailures: Partial<Record<string, NotebooksFailure>> = $state({});
  /** How many dropped files each notebook still takes, by its ID. */
  adding: Partial<Record<string, number>> = $state({});
  deleteOpen = $state(false);
  /** The latest deletion confirmation, which stays while it closes. */
  deleting: DeleteRequest | null = $state(null);
  /** The values of the switches whose calls are on their way, by the source's ID. */
  including: Partial<Record<string, boolean>> = $state({});
  /** The latest name dialog, which stays while it closes. */
  name: NameRequest | null = $state(null);
  nameOpen = $state(false);
  paste: PasteRequest | null = $state(null);
  pasteOpen = $state(false);
  /** The notebook whose file dialog is open, by its ID. */
  picking: string | null = $state(null);
  /** The files that the latest additions refused, by the notebook's ID, until the owner dismisses them. */
  refusals: Partial<Record<string, FileRefusal[]>> = $state({});
  removeOpen = $state(false);
  removing: RemoveRequest | null = $state(null);
  /** Why the latest action on a source failed, by the source's ID. */
  sourceFailures: Partial<Record<string, NotebooksFailure>> = $state({});

  /** Closes every dialog and forgets the failures, as the plugin turns off. */
  closeAll(): void {
    this.nameOpen = false;
    this.deleteOpen = false;
    this.pasteOpen = false;
    this.removeOpen = false;
    this.addFailures = {};
    this.refusals = {};
    this.sourceFailures = {};
  }

  /**
   * Deletes the notebook of a confirmation with its own chats, and takes it
   * from the other chats that attached it. The chats go first, so that a
   * chat that could not be deleted keeps its notebook.
   */
  async confirmDelete(request: DeleteRequest): Promise<void> {
    request.deleting = true;
    request.failure = null;
    request.chatFailure = null;
    const notebookId = request.notebook.id;
    const chatFailure = await this.#deleteChats(notebookId);
    if (chatFailure !== null) {
      request.deleting = false;
      request.chatFailure = chatFailure;
      return;
    }
    const outcome = await call(() => commands.deleteNotebook({ notebookId }));
    const isGone = !outcome.ok && failureCode(outcome.failure) === 'notebook_not_found';
    if (!outcome.ok && !isGone) {
      request.deleting = false;
      request.failure = outcome.failure;
      return;
    }
    // The core keeps a conversation's attachments until they are detached,
    // so the notebook leaves each chat that used it.
    const detachFailure = await this.#detachEverywhere(notebookId);
    request.deleting = false;
    if (shownNotebookId() === notebookId) {
      await openNotebooks();
    }
    if (detachFailure !== null) {
      request.detachFailure = detachFailure;
      return;
    }
    if (this.deleting === request) {
      this.deleteOpen = false;
    }
  }

  /** Creates or renames the notebook of a name dialog, which closes once the plugin kept it. */
  async confirmName(request: NameRequest): Promise<void> {
    request.saving = true;
    request.failure = null;
    const name = request.name.trim();
    const renamed = request.notebook;
    const outcome = await call(() =>
      renamed === null ? commands.createNotebook({ name }) : commands.renameNotebook({ name, notebookId: renamed.id }),
    );
    request.saving = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }
    notebookLibrary.keep(outcome.data);
    if (this.name === request) {
      this.nameOpen = false;
    }
    if (renamed === null) {
      await openNotebook(outcome.data.id);
    }
  }

  /** Adds the text of "Paste text", which closes once the plugin kept it. */
  async confirmPaste(request: PasteRequest): Promise<void> {
    request.saving = true;
    request.failure = null;
    const outcome = await call(() =>
      commands.addTextSource({ notebookId: request.notebook.id, text: request.text, title: request.title.trim() }),
    );
    request.saving = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }
    if (this.paste === request) {
      this.pasteOpen = false;
    }
  }

  /** Removes the source of a confirmation, which closes once the plugin removed it. */
  async confirmRemove(request: RemoveRequest): Promise<void> {
    request.removing = true;
    request.failure = null;
    const outcome = await call(() => commands.removeSource({ sourceId: request.source.id }));
    request.removing = false;
    const isGone = !outcome.ok && failureCode(outcome.failure) === 'source_not_found';
    if (!outcome.ok && !isGone) {
      request.failure = outcome.failure;
      return;
    }
    if (this.removing === request) {
      this.removeOpen = false;
    }
  }

  /** Forgets the files that a notebook refused, once the owner read why. */
  dismissRefusals(notebookId: string): void {
    const { [notebookId]: _dismissed, ...others } = this.refusals;
    this.refusals = others;
  }

  /** Adds files that the owner dropped on a notebook's page, one by one. */
  async drop(notebook: NotebookView, files: readonly File[]): Promise<void> {
    if (files.length === 0) {
      return;
    }
    this.#clearAddFailure(notebook.id);
    this.#count(notebook.id, files.length);
    const refused: FileRefusal[] = [];
    for (const file of files) {
      const refusal = await this.#addFile(notebook.id, file);
      this.#count(notebook.id, -1);
      if (refusal !== null) {
        refused.push(refusal);
      }
    }
    this.#refuse(notebook.id, refused);
  }

  /** Whether a source's switch shows it as used in answers, also while its call is on its way. */
  isIncluded(source: SourceView): boolean {
    return this.including[source.id] ?? source.included;
  }

  /** Starts a new chat of the notebook, which belongs to it (ADR-0051), and opens its row, where the chat will show. */
  newChat(notebook: NotebookView): Promise<void> {
    panelRows.set(notebook.id, true);
    return startConversation({ home: { key: notebook.id, label: notebook.name, provider: PLUGIN_ID } });
  }

  /** Opens the system's file dialog and adds the chosen files to a notebook. */
  async pick(notebook: NotebookView): Promise<void> {
    this.#clearAddFailure(notebook.id);
    this.picking = notebook.id;
    const outcome = await call(() =>
      commands.pickSources({ filterName: m.notebooks_dialog_filter(), notebookId: notebook.id }),
    );
    this.picking = null;
    if (!outcome.ok) {
      this.addFailures = { ...this.addFailures, [notebook.id]: outcome.failure };
      return;
    }
    this.#refuse(notebook.id, outcome.data.refused);
  }

  /** Indexes again a source whose indexing failed. */
  async retry(source: SourceView): Promise<void> {
    this.#clearSourceFailure(source.id);
    const outcome = await call(() => commands.retrySource({ sourceId: source.id }));
    if (!outcome.ok) {
      this.#failSource(source.id, outcome.failure);
      return;
    }
    notebookLibrary.replaceSource(outcome.data);
  }

  /** Switches whether a source's passages are searched. */
  async setIncluded(source: SourceView, included: boolean): Promise<void> {
    this.#clearSourceFailure(source.id);
    this.including = { ...this.including, [source.id]: included };
    const outcome = await call(() => commands.setSourceIncluded({ included, sourceId: source.id }));
    if (outcome.ok) {
      notebookLibrary.replaceSource(outcome.data);
    } else {
      this.#failSource(source.id, outcome.failure);
    }
    const { [source.id]: _settled, ...others } = this.including;
    this.including = others;
  }

  /** Opens the name dialog for a new notebook. */
  startCreate(): void {
    this.name = new NameRequest(null);
    this.nameOpen = true;
  }

  /** Opens the deletion confirmation of a notebook, which counts its own chats and the others that use it. */
  startDelete(notebook: NotebookView): void {
    this.deleting = new DeleteRequest(notebook, chatsOf(notebook.id).length, chatsUsing(notebook.id).length);
    this.deleteOpen = true;
  }

  /** Opens "Paste text" for a notebook. */
  startPaste(notebook: NotebookView): void {
    this.paste = new PasteRequest(notebook);
    this.pasteOpen = true;
  }

  /** Opens the removal confirmation of a source. */
  startRemove(source: SourceView): void {
    this.removing = new RemoveRequest(source);
    this.removeOpen = true;
  }

  /** Opens the name dialog for a notebook to rename. */
  startRename(notebook: NotebookView): void {
    this.name = new NameRequest(notebook);
    this.nameOpen = true;
  }

  /**
   * Adds one dropped file, of any size, in parts (plugin ADR-0002), and
   * returns why it was refused, or `null` once the plugin took it.
   */
  async #addFile(notebookId: string, file: File): Promise<FileRefusal | null> {
    // A file that the plugin would refuse never crosses IPC.
    if (!takesDocument(file.name)) {
      return { name: file.name, reason: 'unsupported_type' };
    }
    const started = await call(() => commands.startUpload({ name: file.name, notebookId }));
    if (!started.ok) {
      return { name: file.name, reason: refusalOf(started.failure) ?? started.failure };
    }
    const { uploadId } = started.data;
    const partFailure = await this.#sendParts(uploadId, file);
    if (partFailure !== null) {
      // The plugin drops the parts that arrived; a failure here changes nothing.
      await call(() => commands.cancelUpload({ uploadId }));
      return { name: file.name, reason: partFailure };
    }
    const finished = await call(() => commands.finishUpload({ uploadId }));
    if (finished.ok) {
      return null;
    }
    return { name: file.name, reason: refusalOf(finished.failure) ?? finished.failure };
  }

  #clearAddFailure(notebookId: string): void {
    const { [notebookId]: _cleared, ...others } = this.addFailures;
    this.addFailures = others;
  }

  #clearSourceFailure(sourceId: string): void {
    const { [sourceId]: _cleared, ...others } = this.sourceFailures;
    this.sourceFailures = others;
  }

  #count(notebookId: string, change: number): void {
    const count = (this.adding[notebookId] ?? 0) + change;
    if (count > 0) {
      this.adding = { ...this.adding, [notebookId]: count };
      return;
    }
    const { [notebookId]: _done, ...others } = this.adding;
    this.adding = others;
  }

  /** Deletes a notebook's own chats, and returns why one stayed, or `null` once all are gone. */
  async #deleteChats(notebookId: string): Promise<string | null> {
    for (const chat of chatsOf(notebookId)) {
      const outcome = await deleteConversation(chat.id);
      // A chat that is gone already needs nothing more.
      const isGone =
        !outcome.ok && outcome.failure.kind === 'core' && outcome.failure.error.code === 'conversation_not_found';
      if (!outcome.ok && !isGone) {
        return conversationFailureMessage(outcome.failure, NOTEBOOKS_CONTEXT);
      }
    }
    return null;
  }

  /** Detaches a deleted notebook from the other chats, and returns why one kept it, or `null`. */
  async #detachEverywhere(notebookId: string): Promise<string | null> {
    const chats = chatsUsing(notebookId);
    let failure: string | null = null;
    for (const chat of chats) {
      const outcome = await detach(chat.id, { key: notebookId, provider: PLUGIN_ID });
      // The first failure says why; the other chats are still tried.
      const isFirstFailure = !outcome.ok && failure === null;
      if (isFirstFailure) {
        failure = conversationFailureMessage(outcome.failure, NOTEBOOKS_CONTEXT);
      }
    }
    return failure;
  }

  #failSource(sourceId: string, failure: NotebooksFailure): void {
    this.sourceFailures = { ...this.sourceFailures, [sourceId]: failure };
  }

  /** Sends a file's parts in order, and returns why one could not be read or sent, or `null` once all arrived. */
  async #sendParts(uploadId: string, file: File): Promise<NotebooksFailure | SourceRefusal | null> {
    let index = 0;
    for (let start = 0; start < file.size; start += UPLOAD_PART_BYTES) {
      let data: string;
      try {
        data = await readAsBase64(file.slice(start, start + UPLOAD_PART_BYTES));
      } catch {
        return 'unreadable';
      }
      const part = index;
      const sent = await call(() => commands.addUploadPart({ data, index: part, uploadId }));
      if (!sent.ok) {
        return refusalOf(sent.failure) ?? sent.failure;
      }
      index += 1;
    }
    return null;
  }

  #refuse(notebookId: string, refused: readonly FileRefusal[]): void {
    if (refused.length === 0) {
      return;
    }
    const earlier = this.refusals[notebookId] ?? [];
    this.refusals = { ...this.refusals, [notebookId]: [...earlier, ...refused] };
  }
}

/** The owner's actions on notebooks and sources. */
export const notebookActions = new NotebookActions();
