// The performance suite (NFR-PERF-01 to 04): the end-to-end setup with its
// own specs, which `task e2e:perf` runs on the reference machine.
import { config as endToEnd } from './wdio.conf.ts';

export const config: WebdriverIO.Config = {
  ...endToEnd,
  specs: ['./perf/**/*.test.ts'],
};
