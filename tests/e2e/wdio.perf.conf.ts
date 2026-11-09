// The performance suite (NFR-PERF-01 to 10): the end-to-end setup with its
// own specs, which `task e2e:perf` runs on the reference machine.
// `MANNIE_PERF_PROFILE=low-end` runs the terminal's specs under the low-end
// laptop profile (support/low-end.ts).
import { config as endToEnd } from './wdio.conf.ts';

export const config: WebdriverIO.Config = {
  ...endToEnd,
  // The measurements run whole, whatever part of the suite a run takes.
  shard: undefined,
  specs: ['./perf/**/*.test.ts'],
};
