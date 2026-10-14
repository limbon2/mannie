// Types of the chat feature that the generated bindings do not cover.
import type { ChatError, ProviderProfileError } from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';

/** Why a chat command failed. */
export type ChatFailure = Failure<ChatError>;

/** Why a provider profile command failed. */
export type ProfileFailure = Failure<ProviderProfileError>;
