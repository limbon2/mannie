// Calls of the core's commands (ADR-0007): the result becomes the command's
// data or a failure, and a failure that is not one of the command's errors,
// such as a refused or malformed call, counts as the core not answering.
import { Channel } from '@tauri-apps/api/core';

import type { ChannelBatch } from '#lib/generated/bindings.js';
import type { Outcome } from '#lib/types/calls.js';

type CommandResult<T, E> = { data: T; status: 'ok' } | { error: E; status: 'error' };

function isCodedError(value: unknown): boolean {
  if (typeof value !== 'object') {
    return false;
  }
  if (value === null) {
    return false;
  }
  return 'code' in value;
}

/** Calls a command and returns its data, or why it failed. */
export async function call<T, E>(command: () => Promise<CommandResult<T, E>>): Promise<Outcome<T, E>> {
  try {
    const result = await command();
    if (result.status === 'ok') {
      return { data: result.data, ok: true };
    }
    if (isCodedError(result.error)) {
      return { failure: { error: result.error, kind: 'core' }, ok: false };
    }
    return { failure: { kind: 'ipc' }, ok: false };
  } catch {
    return { failure: { kind: 'ipc' }, ok: false };
  }
}

/**
 * A channel that hands over the items of its batches in order. A batch whose
 * first sequence number does not follow the last one means that batches were
 * lost, and `onGap` asks the subscriber to subscribe again from its cursor.
 */
export function batchChannel<T>(onItems: (items: readonly T[]) => void, onGap: () => void): Channel<ChannelBatch<T>> {
  let expected = 0;
  return new Channel<ChannelBatch<T>>((batch) => {
    if (batch.firstSequence !== expected) {
      onGap();
      return;
    }
    expected = batch.firstSequence + batch.items.length;
    onItems(batch.items);
  });
}
