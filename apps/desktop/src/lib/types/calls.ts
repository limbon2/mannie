// The outcome of a call of the core's commands (ADR-0007), for every feature.

/** Why a command failed: an error the core reported, or no answer at all. */
export type Failure<E> = { readonly error: E; readonly kind: 'core' } | { readonly kind: 'ipc' };

/** The outcome of a command: its data, or why it failed. */
export type Outcome<T, E> =
  { readonly data: T; readonly ok: true } | { readonly failure: Failure<E>; readonly ok: false };
