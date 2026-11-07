// How long ago something happened, in the owner's locale (AGENTS.md: dates and
// numbers through Intl).

const SECOND = 1000;
const MINUTE = 60 * SECOND;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
const WEEK = 7 * DAY;

/** The largest unit that fits, longest first. */
const UNITS: readonly (readonly [Intl.RelativeTimeFormatUnit, number])[] = [
  ['week', WEEK],
  ['day', DAY],
  ['hour', HOUR],
  ['minute', MINUTE],
  ['second', SECOND],
];

const format = new Intl.RelativeTimeFormat(undefined, { numeric: 'always', style: 'narrow' });

/** How long before `now` the moment `at` was, such as "2h ago". */
export function relativeTime(at: number, now: number): string {
  const elapsed = Math.max(0, now - at);
  for (const [unit, size] of UNITS) {
    if (elapsed >= size) {
      return format.format(-Math.floor(elapsed / size), unit);
    }
  }
  // Minus zero is the past for Intl, so this reads "0s ago" and not "in 0s".
  return format.format(-0, 'second');
}
