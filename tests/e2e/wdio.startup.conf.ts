// Check the driver and the app once before scheduling the full suite. A
// broken browser runtime must fail here instead of retrying every spec.
import { config as endToEnd } from './wdio.conf.ts';

export const config: WebdriverIO.Config = {
  ...endToEnd,
  // The check runs before every part of the suite and is no part of it.
  shard: undefined,
  specs: ['./startup.test.ts'],
};
