// @ts-check
import { describe, it } from 'node:test';

import { RuleTester } from 'eslint';
import * as svelteParser from 'svelte-eslint-parser';
import tseslint from 'typescript-eslint';

import { noComplexInlineHandler } from '../../../scripts/eslint/no-complex-inline-handler.mjs';

RuleTester.describe = describe;
RuleTester.it = it;
RuleTester.itOnly = it.only;

const languageOptions = { parser: svelteParser, parserOptions: { parser: tseslint.parser } };
const tester = new RuleTester({ languageOptions });

/** @param {string} markup */
function component(markup) {
  return { code: `<script lang="ts"></script>\n${markup}`, filename: 'Component.svelte' };
}

tester.run('no-complex-inline-handler', noComplexInlineHandler, {
  valid: [
    component('<button onclick={save} />'),
    component('<button onclick={condition ? save : cancel} />'),
    component('<button onclick={preventDefault(save)} />'),
    component('<button onclick={() => save(getPath())} />'),
    component('<button onclick={() => (open = true)} />'),
    component('<button onclick={() => count++} />'),
    component('<button onclick={() => void listener?.()} />'),
    component('<button onclick={() => save(condition ? path : fallback)} />'),
    component('<Widget onOpenChange={(open) => setOpen(open)} />'),
    component('<Widget onOpenChange={(setOpen as Handler)!} />'),
    component('<Widget render={() => { return label; }} />'),
    component('<Widget OnRender={() => { return label; }} />'),
    component('<button on:click={save} />'),
    component('<button on:click />'),
  ],
  invalid: [
    '<button onclick={() => { save(); close(); }} />',
    '<button onclick={function () { save(); }} />',
    '<button onclick={async () => save()} />',
    '<button onclick={async () => await save()} />',
    '<button onclick={() => ready && save()} />',
    '<button onclick={() => ready ? save() : cancel()} />',
    '<button onclick={() => save(ready ? getPath() : fallback)} />',
    '<button onclick={() => save((ready = true))} />',
    '<button onclick={() => (save(), close())} />',
    '<button onclick={() => tasks.map((task) => save(task))} />',
    '<button onclick={ready ? (() => { save(); close(); }) : cancel} />',
    '<button onclick={preventDefault(() => { save(); close(); })} />',
    '<button onclick={({...listener, click: () => { save(); close(); }}).click} />',
    '<button onclick={((() => { save(); close(); }) as Handler)!} />',
    '<button onclick={(() => { save(); close(); }) satisfies Handler} />',
    '<Widget onOpenChange={(open) => { record(open); setOpen(open); }} />',
    '<button on:click={() => { save(); close(); }} />',
  ].map((markup) => ({
    ...component(markup),
    errors: [{ messageId: 'useNamedHandler' }],
  })),
});
