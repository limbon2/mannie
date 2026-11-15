// @ts-check
import { describe, it } from 'node:test';

import { RuleTester } from 'eslint';

import { multilineObjectProperties } from '../../../scripts/eslint/multiline-object-properties.mjs';

RuleTester.describe = describe;
RuleTester.it = it;
RuleTester.itOnly = it.only;

const tester = new RuleTester();
const wrapCallbacks = [{ wrapCallbacks: true }];

tester.run('multiline-object-properties', multilineObjectProperties, {
  valid: [
    'const options = { first: 1, second: 2 };',
    'const options = {\n  first: 1,\n  ...more,\n};',
    'const options = {\n  nested: { first: 1, second: 2 },\n};',
    'const options = { text: question(), action: () => clear() };',
    { code: 'const options = { first: 1, second: 2 };', options: wrapCallbacks },
    { code: 'const options = { text: question(), action: clear };', options: wrapCallbacks },
    { code: 'const options = { action: () => clear() };', options: wrapCallbacks },
  ],
  invalid: [
    {
      code: 'const options = {\n  first: 1, ...more,\n};',
      output: 'const options = {\n  first: 1,\n  ...more,\n};',
      errors: [{ messageId: 'newLine' }],
    },
    {
      code: 'const options = {\r\n  first: 1, second: 2,\r\n};',
      output: 'const options = {\r\n  first: 1,\r\n  second: 2,\r\n};',
      errors: [{ messageId: 'newLine' }],
    },
    {
      code: 'const options = {\n  first: 1, /* comment */ second: 2,\n};',
      output: null,
      errors: [{ messageId: 'newLine' }],
    },
    {
      code: `function confirmClear() {
  client.confirmation = { text: m.api_clear_cookies_question(), action: () => change(null, true) };
}`,
      output: `function confirmClear() {
  client.confirmation = {
    text: m.api_clear_cookies_question(),
    action: () => change(null, true)
  };
}`,
      options: wrapCallbacks,
      errors: [{ messageId: 'wrapCallback' }],
    },
    {
      code: 'function make() {\r\n  const result = { text: question(), action: function () { clear(); } };\r\n}',
      output:
        'function make() {\r\n  const result = {\r\n    text: question(),\r\n    action: function () { clear(); }\r\n  };\r\n}',
      options: wrapCallbacks,
      errors: [{ messageId: 'wrapCallback' }],
    },
    {
      code: 'const options = { text: question(), /* explains action */ action: () => clear() };',
      output: null,
      options: wrapCallbacks,
      errors: [{ messageId: 'wrapCallback' }],
    },
  ],
});
