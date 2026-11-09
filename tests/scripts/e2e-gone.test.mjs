import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';

import { waitForGone } from '../e2e/support/page.ts';

const NAMES = ['$', 'browser', 'document'];
const originals = NAMES.map((name) => Object.getOwnPropertyDescriptor(globalThis, name));

afterEach(() => {
  for (const [index, name] of NAMES.entries()) {
    const original = originals[index];
    if (original === undefined) {
      Reflect.deleteProperty(globalThis, name);
    } else {
      Object.defineProperty(globalThis, name, original);
    }
  }
});

/**
 * A page whose selector matches for the first looks, and a driver that looks
 * at most five times and fails when it is asked for an element.
 */
function pageThatMatches(times) {
  const asked = [];
  const define = (name, value) => {
    Object.defineProperty(globalThis, name, { configurable: true, value });
  };
  define('document', {
    querySelector: (selector) => {
      asked.push(selector);
      return asked.length > times ? null : {};
    },
  });
  define('$', () => {
    throw new Error('a search of the driver can end with a stale element reference');
  });
  define('browser', {
    execute: async (script, ...parameters) => script(...parameters),
    waitUntil: async (condition, options) => {
      for (let look = 0; look < 5; look += 1) {
        if (await condition()) {
          return;
        }
      }
      throw new Error(options.timeoutMsg);
    },
  });
  return asked;
}

test('an element that leaves the page is gone, without a search of the driver', async () => {
  const asked = pageThatMatches(2);
  await waitForGone('[data-row="one"]');
  assert.deepEqual(asked, ['[data-row="one"]', '[data-row="one"]', '[data-row="one"]']);
});

test('an element that was never there is gone at the first look', async () => {
  const asked = pageThatMatches(0);
  await waitForGone('#missing');
  assert.deepEqual(asked, ['#missing']);
});

test('an element that stays fails with its selector', async () => {
  pageThatMatches(Number.POSITIVE_INFINITY);
  await assert.rejects(waitForGone('[data-row="one"]'), /\[data-row="one"\] stayed on the page/u);
});
