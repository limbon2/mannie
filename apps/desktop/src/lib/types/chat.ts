// Types of the chat feature that the generated bindings do not cover.
import type { ChatError, ChatGptError, ProviderProfileError } from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';

/** Why a chat command failed. */
export type ChatFailure = Failure<ChatError>;

/** Why a command of the ChatGPT account, or a sign-in, failed (ADR-0062). */
export type ChatGptFailure = Failure<ChatGptError>;

/** Why a provider profile command failed. */
export type ProfileFailure = Failure<ProviderProfileError>;
