import { commands, type ConversationId, type ExportError, type ExportOutcome } from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';
import { call } from '#lib/utils/calls.js';

/** The owner's native export action and its result dialog. */
export class ExportActions {
  busy = $state(false);
  open = $state(false);
  conversationId: ConversationId | null = $state(null);
  result: Extract<ExportOutcome, { kind: 'saved' }> | null = $state(null);
  failure: Failure<ExportError> | null = $state(null);

  async start(conversationId: ConversationId): Promise<void> {
    if (this.busy) {
      return;
    }
    this.busy = true;
    this.open = false;
    this.conversationId = conversationId;
    this.result = null;
    this.failure = null;
    try {
      const outcome = await call(() => commands.exportConversation({ conversationId }));
      if (outcome.ok) {
        if (outcome.data.kind === 'saved') {
          this.result = outcome.data;
          this.open = true;
        }
      } else {
        this.failure = outcome.failure;
        this.open = true;
      }
    } finally {
      this.busy = false;
    }
  }
}

export const exportActions = new ExportActions();
