// @ts-check
import { describe, it } from 'node:test';

import stylistic from '@stylistic/eslint-plugin';
import { RuleTester } from 'eslint';
import * as svelteParser from 'svelte-eslint-parser';
import tseslint from 'typescript-eslint';

import { statementGroupPadding } from '../../../scripts/eslint/statement-group-spacing.mjs';

RuleTester.describe = describe;
RuleTester.it = it;
RuleTester.itOnly = it.only;

const tester = new RuleTester();
const options = [...statementGroupPadding];
const expectedBlankLine = { messageId: 'expectedBlankLine' };

tester.run('@stylistic/padding-line-between-statements', stylistic.rules['padding-line-between-statements'], {
  valid: [
    {
      options,
      code: `async function change() {
  const first = readFirst();
  let second = readSecond();

  pending = true;
  attempts++;

  await save(first);
  await save(second);
  return second;
}`,
    },
    {
      options,
      code: `function cleanupOnly() {
  return () => close();
}`,
    },
  ],
  invalid: [
    {
      filename: 'Component.svelte',
      languageOptions: { parser: svelteParser, parserOptions: { parser: tseslint.parser } },
      options,
      code: `<script lang="ts">
  async function change(): Promise<void> {
    const current = read();
    const next = nextValue(current);
    pending = true;
    status = 'saving';
    try {
      status = 'pending';
      await save(next);
      await notify();
      revision += 1;
      changes++;
    } finally {
      pending = false;
    }
  }

  $effect(() => {
    void load();
    return () => {
      dispose();
    };
  });
</script>`,
      output: `<script lang="ts">
  async function change(): Promise<void> {
    const current = read();
    const next = nextValue(current);

    pending = true;
    status = 'saving';

    try {
      status = 'pending';

      await save(next);
      await notify();

      revision += 1;
      changes++;
    } finally {
      pending = false;
    }
  }

  $effect(() => {
    void load();

    return () => {
      dispose();
    };
  });
</script>`,
      errors: [expectedBlankLine, expectedBlankLine, expectedBlankLine, expectedBlankLine, expectedBlankLine],
    },
    {
      options,
      code: `function subscribe() {
  connect(); // Starts a session that belongs to this subscriber.
  // An unsubscribe can run after the connection closes remotely.
  return function unsubscribe() {
    disconnect();
  };
}`,
      output: `function subscribe() {
  connect(); // Starts a session that belongs to this subscriber.

  // An unsubscribe can run after the connection closes remotely.
  return function unsubscribe() {
    disconnect();
  };
}`,
      errors: [expectedBlankLine],
    },
    {
      options,
      code: `function start() {
  const job = prepare();
  attempts++;
  run(job);
}`,
      output: `function start() {
  const job = prepare();

  attempts++;
  run(job);
}`,
      errors: [expectedBlankLine],
    },
  ],
});
