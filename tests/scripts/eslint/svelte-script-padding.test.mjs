// @ts-check
import { describe, it } from 'node:test';

import { RuleTester } from 'eslint';
import * as svelteParser from 'svelte-eslint-parser';
import tseslint from 'typescript-eslint';

import { svelteScriptPadding } from '../../../scripts/eslint/svelte-script-padding.mjs';

RuleTester.describe = describe;
RuleTester.it = it;
RuleTester.itOnly = it.only;

const languageOptions = { parser: svelteParser, parserOptions: { parser: tseslint.parser } };
const tester = new RuleTester({ languageOptions });

/** @param {string} body The component's script body with its indentation. */
function script(body) {
  return `<script lang="ts">\n${body}\n</script>`;
}

const filename = 'Component.svelte';
const blankLine = { messageId: 'blankLine' };

tester.run('mannie/svelte-script-padding', svelteScriptPadding, {
  valid: [
    {
      filename,
      code: script(`  import { first } from './first';
  import { second } from './second';

  let { title } = $props();
  const id = $props.id();

  const total = $derived(first + second);`),
    },
    {
      filename,
      code: script(`  const first = 1;
  const second = 2;

  function sum(): number {
    return first + second;
  }`),
    },
    {
      filename,
      code: script(`  let failure = $state('');
  const cookies = $state.raw<CookieView[]>([]);

  let generation = 0;
  let previousGeneration = 0;

  const firstPage = 1;
  const pageSize = 25;`),
    },
  ],
  invalid: [
    {
      filename,
      code: script(`  let cookies: CookieView[] = $state([]);
  let value = $state('');
  let loading = $state(false);
  let changing = $state(false);
  let failure = $state('');
  let generation = 0;
  const dates = new Intl.DateTimeFormat(undefined, { dateStyle: 'short', timeStyle: 'medium' });`),
      output: script(`  let cookies: CookieView[] = $state([]);
  let value = $state('');
  let loading = $state(false);
  let changing = $state(false);
  let failure = $state('');

  let generation = 0;

  const dates = new Intl.DateTimeFormat(undefined, { dateStyle: 'short', timeStyle: 'medium' });`),
      errors: [
        { ...blankLine, line: 7 },
        { ...blankLine, line: 8 },
      ],
    },
    {
      filename,
      code: script(`  import { first } from './first'; // Preserve the trailing note.
  // Keep the leading note with the following declaration.
  const total = first;`),
      output: script(`  import { first } from './first'; // Preserve the trailing note.

  // Keep the leading note with the following declaration.
  const total = first;`),
      errors: [blankLine],
    },
    {
      filename,
      code: script(`  let { title } = $props();
  const id = $props.id();
  const ready = $state(false);`),
      output: script(`  let { title } = $props();
  const id = $props.id();

  const ready = $state(false);`),
      errors: [blankLine],
    },
    {
      filename,
      code: script(`  const ready = $state(false);
  /* A blank line inside the comment is not a separator.

   * Keep this note with the function.
   */
  function isReady(): boolean {
    return ready;
  }
  $effect(() => sync(isReady()));`),
      output: script(`  const ready = $state(false);

  /* A blank line inside the comment is not a separator.

   * Keep this note with the function.
   */
  function isReady(): boolean {
    return ready;
  }

  $effect(() => sync(isReady()));`),
      errors: [blankLine, blankLine],
    },
    {
      filename,
      code: `<script lang="ts" module>\r\n  const ready = true;\r\n  export function isReady() { return ready; }\r\n</script>`,
      output: `<script lang="ts" module>\r\n  const ready = true;\r\n\r\n  export function isReady() { return ready; }\r\n</script>`,
      errors: [blankLine],
    },
  ],
});
