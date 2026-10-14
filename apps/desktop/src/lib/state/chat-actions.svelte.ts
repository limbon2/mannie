// What the owner does with conversations from more than one place: the
// command palette, the conversation list and the conversation view. The
// dialogs these actions open are mounted once, as the chat feature's overlay.
import {
  type ChatError,
  type Checkpoint,
  type CheckpointId,
  commands,
  type ConversationId,
  type ConversationSummary,
  type ProfileId,
} from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';
import { call } from '#lib/utils/calls.js';
import { openConversationId, startNewConversation } from '#lib/utils/chat-urls.js';

/** The checkpoint whose summary is shown. */
export interface CheckpointTarget {
  readonly checkpointId: CheckpointId;
  readonly conversationId: ConversationId;
}

/** The checkpoint summary in its dialog: loading, loaded, or why it could not be read. */
export type ShownCheckpoint =
  | { readonly checkpoint: Checkpoint; readonly status: 'loaded' }
  | { readonly failure: Failure<ChatError>; readonly status: 'failed' }
  | { readonly status: 'loading' };

/** One opening of the delete confirmation. */
export class DeleteRequest {
  readonly conversation: ConversationSummary;
  deleting = $state(false);
  /** Why the core refused the latest attempt. */
  failure: Failure<ChatError> | null = $state(null);

  constructor(conversation: ConversationSummary) {
    this.conversation = conversation;
  }
}

/** One opening of the rename dialog, which starts from the current title. */
export class RenameRequest {
  readonly conversation: ConversationSummary;
  /** Why the core refused the latest title. */
  failure: Failure<ChatError> | null = $state(null);
  saving = $state(false);
  title = $state('');

  constructor(conversation: ConversationSummary) {
    this.conversation = conversation;
    this.title = conversation.title;
  }
}

/** The owner's actions on conversations, and the dialogs they open. */
export class ChatActions {
  /** The checkpoint summary dialog's content, which stays while the dialog closes. */
  checkpoint: ShownCheckpoint = $state.raw({ status: 'loading' });
  /** Whether the checkpoint summary dialog is open. */
  checkpointOpen = $state(false);
  /** Compactions the owner started that have not ended, by conversation. */
  compacting: ConversationId[] = $state([]);
  /** Why the owner's latest compaction of a conversation failed. */
  compactionFailures: Partial<Record<ConversationId, Failure<ChatError>>> = $state({});
  /** Whether the delete confirmation is open. */
  deleteOpen = $state(false);
  /** The latest delete confirmation, which stays while its dialog closes. */
  deleting: DeleteRequest | null = $state(null);
  /** Whether the rename dialog is open. */
  renameOpen = $state(false);
  /** The latest rename dialog, which stays while it closes. */
  renaming: RenameRequest | null = $state(null);
  /** Counts the checkpoints the owner opened, so that only the latest one is shown. */
  #checkpointRequests = 0;

  /** Compacts a conversation now. */
  async compact(id: ConversationId): Promise<void> {
    if (this.compacting.includes(id)) {
      return;
    }
    this.compacting = [...this.compacting, id];
    const { [id]: _previous, ...others } = this.compactionFailures;
    this.compactionFailures = others;
    const outcome = await call(() => commands.compactConversation({ conversationId: id }));
    this.compacting = this.compacting.filter((compacting) => compacting !== id);
    if (!outcome.ok) {
      this.compactionFailures = { ...this.compactionFailures, [id]: outcome.failure };
    }
  }

  /**
   * Deletes the conversation of a delete confirmation, which closes when the
   * core deleted it. An open conversation is left for a new one.
   */
  async confirmDelete(request: DeleteRequest): Promise<void> {
    const id = request.conversation.id;
    request.deleting = true;
    const outcome = await call(() => commands.deleteConversation({ conversationId: id }));
    request.deleting = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }
    if (this.deleting === request) {
      this.deleteOpen = false;
    }
    if (openConversationId() === id) {
      await startNewConversation();
    }
  }

  /** Forgets why a compaction failed, once the owner has read it. */
  dismissCompactionFailure(id: ConversationId): void {
    const { [id]: _dismissed, ...others } = this.compactionFailures;
    this.compactionFailures = others;
  }

  /** Starts a new run for the latest turn. */
  async retry(id: ConversationId): Promise<Failure<ChatError> | null> {
    const outcome = await call(() => commands.retryReply({ conversationId: id }));
    return outcome.ok ? null : outcome.failure;
  }

  /** Saves the title of a rename dialog, which closes when the core accepted it. */
  async saveRename(request: RenameRequest): Promise<void> {
    request.saving = true;
    const outcome = await call(() =>
      commands.renameConversation({ conversationId: request.conversation.id, title: request.title }),
    );
    request.saving = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }
    request.failure = null;
    if (this.renaming === request) {
      this.renameOpen = false;
    }
  }

  /** Chooses the profile of a conversation's next replies, or the default for `null`. */
  async setProfile(id: ConversationId, profileId: ProfileId | null): Promise<Failure<ChatError> | null> {
    const outcome = await call(() => commands.setConversationProfile({ conversationId: id, profileId }));
    return outcome.ok ? null : outcome.failure;
  }

  /** Opens the summary of a checkpoint in its dialog and reads it from the core. */
  async showCheckpoint(target: CheckpointTarget): Promise<void> {
    this.#checkpointRequests += 1;
    const request = this.#checkpointRequests;
    this.checkpoint = { status: 'loading' };
    this.checkpointOpen = true;
    const outcome = await call(() => commands.getCheckpoint(target));
    if (request !== this.#checkpointRequests) {
      // The owner opened another checkpoint while this one was read.
      return;
    }
    this.checkpoint = outcome.ok
      ? { checkpoint: outcome.data, status: 'loaded' }
      : { failure: outcome.failure, status: 'failed' };
  }

  /** Asks the owner to confirm the deletion of a conversation. */
  startDelete(conversation: ConversationSummary): void {
    this.deleting = new DeleteRequest(conversation);
    this.deleteOpen = true;
  }

  /** Opens the rename dialog for a conversation. */
  startRename(conversation: ConversationSummary): void {
    this.renaming = new RenameRequest(conversation);
    this.renameOpen = true;
  }

  /** Stops the conversation's reply. */
  async stop(id: ConversationId): Promise<Failure<ChatError> | null> {
    const outcome = await call(() => commands.stopReply({ conversationId: id }));
    return outcome.ok ? null : outcome.failure;
  }
}

/** The window's chat actions. */
export const chatActions = new ChatActions();
