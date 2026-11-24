// @ts-check
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { fetchWithRetry, versionFromRegistry } from '../../scripts/edgedriver.mjs';

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

describe('fetchWithRetry', () => {
  it('retries transient HTTP 5xx and network failures before succeeding', async () => {
    let calls = 0;
    /** @type {number[]} */
    const waits = [];
    const response = await fetchWithRetry('https://example.invalid/driver.zip', {
      attempts: 3,
      delayMillis: 100,
      fetcher: async () => {
        calls += 1;
        if (calls === 1) {
          throw new Error('temporary disconnect');
        }
        if (calls === 2) {
          return new Response('', { status: 503 });
        }
        return new Response('ok', { status: 200 });
      },
      wait: async (millis) => {
        waits.push(millis);
      },
    });

    assert.equal(response.status, 200);
    assert.equal(calls, 3);
    assert.deepEqual(waits, [100, 200]);
  });

  it('stops immediately on non-retryable HTTP 404 responses', async () => {
    let calls = 0;
    await assert.rejects(
      fetchWithRetry('https://example.invalid/missing.zip', {
        attempts: 3,
        delayMillis: 100,
        fetcher: async () => {
          calls += 1;
          return new Response('', { status: 404 });
        },
        wait: async () => {
          throw new Error('should not retry 404');
        },
      }),
      /HTTP 404/u,
    );
    assert.equal(calls, 1);
  });
});
