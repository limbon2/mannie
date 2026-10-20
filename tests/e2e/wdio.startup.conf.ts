// Check the driver and the app once before scheduling the full suite. A
// broken browser runtime must fail here instead of retrying every spec.
import { config as endToEnd } from './wdio.conf.ts';

export const config: WebdriverIO.Config = {
  ...endToEnd,
  specs: ['./startup.test.ts'],
};
