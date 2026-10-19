// The IPC part of the TypeScript plugin SDK, `@mannie/plugin-sdk/ipc`
// (ADR-0034, decisions 9 and 11): what a plugin's UI and its generated
// bindings use to call the plugin's commands and follow its channels. The
// envelopes come from the core's generated `plugin-sdk.ts`, and the calls
// read as the core's own calls do.
import { commands } from '#lib/generated/bindings.js';
import type { CommandError, PluginFailure, SubscriptionId } from '#lib/generated/plugin-sdk.js';
import type { Failure, Outcome } from '#lib/types/calls.js';

export type {
  ChannelBatch,
  ChannelMessage,
  CommandError,
  HostRefusal,
  PluginFailure,
  Result,
  SubscriptionEnd,
  SubscriptionId,
} from '#lib/generated/plugin-sdk.js';
export type { Failure, Outcome } from '#lib/types/calls.js';
export { batchChannel, call } from '#lib/utils/calls.js';
export { Channel } from '@tauri-apps/api/core';

/** Why a call of a plugin's command failed: the host's refusal or the plugin's error, or no answer at all. */
export type PluginCallFailure<E extends { code: string }> = Failure<CommandError<PluginFailure<E>>>;

/** The outcome of a call of a plugin's command. */
export type PluginOutcome<T, E extends { code: string }> = Outcome<T, CommandError<PluginFailure<E>>>;

/**
 * Ends a subscription to one of the plugin's channels. Ending one that
 * already ended changes nothing, and the host ends a plugin's channels
 * itself when the plugin stops.
 */
export function unsubscribe(subscriptionId: SubscriptionId): Promise<void> {
  return commands.unsubscribe({ subscriptionId });
}
