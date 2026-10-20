import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';

import { sendMessage } from '../e2e/support/chat.ts';

const names = ['browser', '$', 'document', 'getComputedStyle'];
const originals = new Map(names.map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]));

afterEach(() => {
  for (const [name, original] of originals) {
    if (original === undefined) {
      Reflect.deleteProperty(globalThis, name);
    } else {
      Object.defineProperty(globalThis, name, original);
    }
  }
});

function conversation({ acceptsMessage = true } = {}) {
  let turns = 2;
  let sent = false;
  let waits = 0;
  let clicked = false;
  let message = '';
  const box = {
    click: async () => {
      clicked = true;
    },
    disabled: false,
    setValue: async (text) => {
      message = text;
    },
  };
  const values = {
    $: async () => box,
    browser: {
      execute: async (read, ...args) => read(...args),
      keys: async () => {
        assert.equal(clicked, true);
        assert.equal(message, 'Second message');
        sent = true;
      },
      waitUntil: async (condition, options) => {
        for (let attempt = 0; attempt < 2; attempt += 1) {
          if (await condition()) {
            return;
          }
          if (sent) {
            waits += 1;
            if (acceptsMessage) {
              turns += 1;
            }
          }
        }
        throw new Error(options.timeoutMsg);
      },
    },
    document: {
      body: {},
      querySelector: () => box,
      querySelectorAll: () => ({ length: turns }),
    },
    getComputedStyle: () => ({ pointerEvents: 'auto' }),
  };
  for (const [name, value] of Object.entries(values)) {
    Object.defineProperty(globalThis, name, { configurable: true, value });
  }
  return { turnCount: () => turns, waits: () => waits };
}

test('sending waits for a new turn while the previous completed reply remains visible', async () => {
  const page = conversation();
  await sendMessage('Second message');
  assert.equal(page.turnCount(), 3);
  assert.equal(page.waits(), 1);
});

test('sending fails if the new turn never appears', async () => {
  const page = conversation({ acceptsMessage: false });
  await assert.rejects(sendMessage('Second message'), /the message did not show in the transcript/u);
  assert.equal(page.turnCount(), 2);
});
