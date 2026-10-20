import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';

import { waitForShownText } from '../e2e/support/page.ts';

const originalBrowser = Object.getOwnPropertyDescriptor(globalThis, 'browser');
const originalSelector = Object.getOwnPropertyDescriptor(globalThis, '$');

afterEach(() => {
  for (const [name, original] of [
    ['browser', originalBrowser],
    ['$', originalSelector],
  ]) {
    if (original === undefined) {
      Reflect.deleteProperty(globalThis, name);
    } else {
      Object.defineProperty(globalThis, name, original);
    }
  }
});

function elementWith(text, { displayed = true, exists = true } = {}) {
  const element = {
    innerText: text,
    getText: () => {
      throw new Error('WebKit cannot report this truncated text through getText');
    },
    isDisplayed: async () => displayed,
    isExisting: async () => exists,
    selector: '#sample',
  };
  Object.defineProperty(globalThis, '$', { configurable: true, value: async () => element });
  Object.defineProperty(globalThis, 'browser', {
    configurable: true,
    value: {
      execute: async (read, target) => read(target),
      waitUntil: async (condition, options) => {
        if (!(await condition())) {
          throw new Error(options.timeoutMsg);
        }
      },
    },
  });
  return element;
}

test('visible truncated text is checked through the DOM for selectors and elements', async () => {
  const element = elementWith('  A long profile name  ');
  await waitForShownText('#sample', 'A long profile name');
  await waitForShownText(element, /profile/u);
  await waitForShownText(Promise.resolve(element), {
    asymmetricMatch: (value) => {
      const includesProfile = typeof value === 'string' && value.includes('profile');
      return includesProfile;
    },
  });
});

test('a hidden or missing element cannot satisfy a text assertion', async () => {
  for (const state of [{ displayed: false }, { exists: false }]) {
    elementWith('Expected text', state);
    await assert.rejects(waitForShownText('#sample', 'Expected text'), /did not show/u);
  }
});

test('visible text still fails an incorrect exact or partial expectation', async () => {
  elementWith('Different text');
  await assert.rejects(waitForShownText('#sample', 'Expected text'), /did not show/u);
  await assert.rejects(waitForShownText('#sample', /Expected/u), /did not show/u);
});
