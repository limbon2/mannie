// The owner's chat settings: the instructions that apply to every
// conversation (docs/features/chat/conversations.md, the owner's
// instructions).
import { type ChatError, type ChatSettings, commands } from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';
import { call } from '#lib/utils/calls.js';

/** The chat settings and the outcome of the latest request. */
export class ChatSettingsState {
  /** The instructions as stored. */
  instructions = $state('');
  /** Why the settings could not be read, if they could not. */
  readFailure: Failure<ChatError> | null = $state(null);
  /** Why the latest change failed, if it failed. */
  saveFailure: Failure<ChatError> | null = $state(null);
  saving = $state(false);
  status: 'error' | 'loading' | 'ready' = $state('loading');

  /** Reads the settings from the core. */
  async load(): Promise<void> {
    const outcome = await call(() => commands.getChatSettings());
    if (outcome.ok) {
      this.#take(outcome.data);
      this.status = 'ready';
      this.readFailure = null;
    } else {
      this.readFailure = outcome.failure;
      this.status = 'error';
    }
  }

  /** Saves new instructions. Returns whether they were saved. */
  async save(instructions: string): Promise<boolean> {
    this.saving = true;
    this.saveFailure = null;
    const outcome = await call(() => commands.updateChatSettings({ instructions }));
    this.saving = false;
    if (outcome.ok) {
      this.#take(outcome.data);
      return true;
    }
    this.saveFailure = outcome.failure;
    return false;
  }

  #take(settings: ChatSettings): void {
    this.instructions = settings.instructions;
  }
}

/** The window's chat settings. */
export const chatSettings = new ChatSettingsState();
