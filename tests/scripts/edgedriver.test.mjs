// @ts-check
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { versionFromRegistry } from '../../scripts/edgedriver.mjs';

describe('versionFromRegistry', () => {
  it('reads the runtime version from reg query', () => {
    const output = [
      '',
      String.raw`HKEY_LOCAL_MACHINE\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}`,
      '    pv    REG_SZ    154.0.4258.62',
      '',
    ].join('\r\n');

    assert.equal(versionFromRegistry(output), '154.0.4258.62');
  });

  it('finds no version in other output', () => {
    assert.equal(
      versionFromRegistry('ERROR: The system was unable to find the specified registry key or value.'),
      undefined,
    );
  });
});
