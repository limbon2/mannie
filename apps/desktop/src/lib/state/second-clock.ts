// A clock for countdowns, such as the wait before a retry. Reading `now` in
// an effect, or in a derived value that an effect reads, runs that effect
// again every second; the timer runs only while something reads the clock.
import { createSubscriber } from 'svelte/reactivity';

/** The current time, updated once a second while it is read. */
export class SecondClock {
  readonly #subscribe = createSubscriber((update) => {
    const timer = setInterval(update, 1_000);
    return () => {
      clearInterval(timer);
    };
  });

  /** Milliseconds since the Unix epoch. */
  get now(): number {
    this.#subscribe();
    return Date.now();
  }
}

/** The window's clock for countdowns. */
export const secondClock = new SecondClock();
