// @ts-check
import { describe, it } from 'node:test';

import { RuleTester } from 'eslint';
import * as svelteParser from 'svelte-eslint-parser';
import tseslint from 'typescript-eslint';

import { namedTypeAnnotations } from '../../../scripts/eslint/named-type-annotations.mjs';

RuleTester.describe = describe;
RuleTester.it = it;
RuleTester.itOnly = it.only;

const tester = new RuleTester({ languageOptions: { parser: tseslint.parser } });
const svelteOptions = { parser: svelteParser, parserOptions: { parser: tseslint.parser } };
const unnamedType = { messageId: 'nameType' };

tester.run('mannie/named-type-annotations', namedTypeAnnotations, {
  valid: [
    {
      code: `<script lang="ts">
  type Props = {
    options: { label: string; value: string }[];
  };

  let { options }: Props = $props();
</script>`,
      filename: 'Select.svelte',
      languageOptions: svelteOptions,
    },
    `interface Payload {
  options: readonly { label: string; value: string }[];
}

function load(payload: Payload): Promise<Payload> {
  return Promise.resolve(payload);
}`,
    `type Row = { path: string };
const rows: readonly (Row | null)[] = [];`,
    `const notify = (message: string): void => {
  send(message);
};`,
  ],
  invalid: [
    {
      code: `<script lang="ts">
  let { collectionId, environmentId }: { collectionId: string; environmentId: string } = $props();
</script>`,
      errors: [{ ...unnamedType, line: 2 }],
      filename: 'CookiesView.svelte',
      languageOptions: svelteOptions,
      output: null,
    },
    {
      code: 'const options: Record<string, { label: string }[]> = {};',
      errors: [unnamedType],
      output: null,
    },
    {
      code: `function paths(rows: readonly ({ path: string } | null)[]): number {
  return rows.length;
}`,
      errors: [unnamedType],
      output: null,
    },
    {
      code: `async function save(): Promise<{ saved: boolean }> {
  return { saved: true };
}`,
      errors: [unnamedType],
      output: null,
    },
    {
      code: 'const isReady = ({ ready }: { ready: boolean }): boolean => ready;',
      errors: [unnamedType],
      output: null,
    },
  ],
});
