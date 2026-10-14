// Durations for the owner's locale (docs/features/chat/conversation-view.md,
// parts): the countdown to the next attempt of a reply.

/** Whole seconds from `now` until `at`, never below zero. */
export function secondsUntil(at: number, now: number): number {
  return Math.max(0, Math.ceil((at - now) / 1_000));
}

/** A number of seconds, such as "5 seconds", in the owner's language. */
export function formatSeconds(seconds: number): string {
  return new Intl.NumberFormat(undefined, { style: 'unit', unit: 'second', unitDisplay: 'long' }).format(seconds);
}
