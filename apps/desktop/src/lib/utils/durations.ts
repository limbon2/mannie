// Durations for the owner's locale: the countdown to the next attempt of a
// reply (docs/features/chat/conversation-view.md, parts), and how long a tool
// ran (docs/features/system-tools/tool-activity.md).

/** Whole seconds from `now` until `at`, never below zero. */
export function secondsUntil(at: number, now: number): number {
  return Math.max(0, Math.ceil((at - now) / 1_000));
}

/** A number of seconds, such as "5 seconds", in the owner's language. */
export function formatSeconds(seconds: number): string {
  return new Intl.NumberFormat(undefined, { style: 'unit', unit: 'second', unitDisplay: 'long' }).format(seconds);
}

function unitAmount(value: number, unit: 'hour' | 'minute' | 'second', fractionDigits = 0): string {
  return new Intl.NumberFormat(undefined, {
    maximumFractionDigits: fractionDigits,
    style: 'unit',
    unit,
    unitDisplay: 'short',
  }).format(value);
}

/**
 * A duration in short units, such as "3.2 sec" or "2 min 5 sec". Durations
 * below ten seconds keep a tenth of a second.
 */
export function formatDuration(milliseconds: number): string {
  const seconds = Math.max(0, milliseconds) / 1_000;
  if (seconds < 10) {
    return unitAmount(Math.floor(seconds * 10) / 10, 'second', 1);
  }
  const whole = Math.floor(seconds);
  if (whole < 60) {
    return unitAmount(whole, 'second');
  }
  const hours = Math.floor(whole / 3_600);
  const minutes = Math.floor((whole % 3_600) / 60);
  const rest = whole % 60;
  const parts: string[] = [];
  if (hours > 0) {
    parts.push(unitAmount(hours, 'hour'));
  }
  if (minutes > 0) {
    parts.push(unitAmount(minutes, 'minute'));
  }
  // Seconds matter only below an hour.
  const showsSeconds = rest > 0 && hours === 0;
  if (showsSeconds) {
    parts.push(unitAmount(rest, 'second'));
  }
  return new Intl.ListFormat(undefined, { style: 'narrow', type: 'unit' }).format(parts);
}
