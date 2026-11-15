// @ts-check
import { describe, it } from 'node:test';

import { RuleTester } from 'eslint';
import * as svelteParser from 'svelte-eslint-parser';

import { templateNewlines } from '../../../scripts/eslint/template-newlines.mjs';

RuleTester.describe = describe;
RuleTester.it = it;
RuleTester.itOnly = it.only;

const tester = new RuleTester({ languageOptions: { parser: svelteParser } });
const filename = 'Component.svelte';

/** @param {string} keyword The Svelte keyword reported by the rule. */
function marker(keyword) {
  return { data: { keyword }, messageId: 'block' };
}

tester.run('mannie/template-newlines', templateNewlines, {
  valid: [
    {
      filename,
      code: `<main>
  {#if show}
    <section>
      <Article />
    </section>
  {:else}
    <Empty />
  {/if}
</main>`,
    },
    { filename, code: '<p>Before {#if show}<em>{word}</em>{/if} after.</p>' },
    { filename, code: 'Before {#if show}<em>{word}</em>{/if} after.' },
    { filename, code: '<span>Before</span>{#if show}<Article />{/if}' },
    { filename, code: '<div><span>One</span><span>Two</span></div>' },
    { filename, code: '<pre><code>{#if show}{word}{/if}</code></pre>' },
    { filename, code: '<div class="whitespace-pre-wrap"><section><Example /></section></div>' },
    { filename, code: '<p><Prose /></p>' },
    { filename, code: '<div>Example {#if show}{@render note()}{/if}.</div>' },
  ],
  invalid: [
    {
      filename,
      code: '{#if show}<Article />{/if}\n<button type="button">Save</button>',
      errors: [marker('#if'), marker('/if')],
    },
    {
      filename,
      code: '<main><section><Article /></section><Empty /></main>',
      errors: [
        { line: 1, column: 7, messageId: 'nested' },
        { line: 1, column: 16, messageId: 'nested' },
      ],
    },
    {
      filename,
      code: '{#if accepts({ ok: true })}<Article />{:else if next}<Other />{:else}<Empty />{/if}',
      errors: [marker('#if'), marker(':else'), marker(':else'), marker('/if')],
    },
    {
      filename,
      code: '{#await result}<Loading />{:then item}<Article {item} />{:catch error}<ErrorView {error} />{/await}',
      errors: [marker('#await'), marker(':then'), marker(':catch'), marker('/await')],
    },
    {
      filename,
      code: '{#each items as { item }}<Article {item} />{:else}<Empty />{/each}',
      errors: [marker('#each'), marker(':else'), marker('/each')],
    },
    {
      filename,
      code: '{#snippet item()}{#key current}<Article />{/key}{/snippet}',
      errors: [marker('#snippet'), marker('#key'), marker('/key'), marker('/snippet')],
    },
  ],
});
