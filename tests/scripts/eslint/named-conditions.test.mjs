// @ts-check
import { describe, it } from 'node:test';

import { RuleTester } from 'eslint';
import * as svelteParser from 'svelte-eslint-parser';
import tseslint from 'typescript-eslint';

import { namedConditions } from '../../../scripts/eslint/named-conditions.mjs';

RuleTester.describe = describe;
RuleTester.it = it;
RuleTester.itOnly = it.only;

const name = { messageId: 'name' };

new RuleTester().run('named-conditions (JavaScript)', namedConditions, {
  valid: [
    'if (isReady && hasItems) {}',
    'if (index < lines.length) {}',
    'if (!isReady || options.enabled) {}',
    'if (user?.isAdmin && list.includes(id)) {}',
    "if (lines[index].startsWith('|') && isOpen) {}",
    'if (!(isReady && hasItems)) {}',
    'if (items.at(-1) && Array.isArray(value)) {}',
    'async function check() { if ((await exists(path)) && isFile) {} }',
    'while (isTableRowAt(index)) {}',
    "const label = isReady && hasItems ? 'ready' : 'waiting';",
    // An initializer names the whole condition, which keeps a guard intact.
    "const isTableRow = index < lines.length && lines[index].trim().startsWith('|');",
    // A returned || that picks a value is not a condition.
    'function pick(key) { return cache.get(key) || load(key, options.depth + 1); }',
    'items.filter((item) => item.visible && isLarge(item));',
  ],
  invalid: [
    {
      code: "while (index < doc.lines.length && doc.lines[index].trim().startsWith('|')) {}",
      errors: [
        { ...name, column: 8, endColumn: 32 },
        { ...name, column: 36, endColumn: 75 },
      ],
    },
    { code: 'if (count > 0 && isVisible) {}', errors: [{ ...name, column: 5 }] },
    { code: "const label = count > 0 && isVisible ? 'a' : 'b';", errors: [name] },
    {
      code: "function isTableRow(line) { return line !== undefined && line.trim().startsWith('|'); }",
      errors: [name, name],
    },
    { code: 'items.filter((item) => item.size > limit && item.visible);', errors: [name] },
    { code: 'if (!(first === second || isReady)) {}', errors: [name] },
    { code: 'if (isReady && check(first + second)) {}', errors: [name] },
    { code: 'for (let i = 0; i < count && !done; i += 1) {}', errors: [name] },
    { code: 'do {} while (isReady || items.some((item) => item.done));', errors: [name] },
  ],
});

new RuleTester({ languageOptions: { parser: tseslint.parser } }).run('named-conditions (TypeScript)', namedConditions, {
  valid: ['if (value! && (other as boolean)) {}'],
  invalid: [{ code: "if (typeof value === 'string' && isNonEmpty(value)) {}", errors: [name] }],
});

new RuleTester({
  languageOptions: { parser: svelteParser, parserOptions: { parser: tseslint.parser } },
}).run('named-conditions (Svelte)', namedConditions, {
  valid: [
    {
      code: '<script lang="ts">let isEmpty = $derived(count === 0 && !loading);</script>{#if isEmpty && isVisible}<p></p>{/if}',
      filename: 'List.svelte',
    },
  ],
  invalid: [
    { code: '{#if count > 0 && isVisible}<p></p>{/if}', errors: [name], filename: 'List.svelte' },
    {
      code: '{#if isReady}<p></p>{:else if count > 0 && isVisible}<p></p>{/if}',
      errors: [name],
      filename: 'List.svelte',
    },
    { code: '<script lang="ts">if (count > 0 && isVisible) {}</script>', errors: [name], filename: 'List.svelte' },
  ],
});
