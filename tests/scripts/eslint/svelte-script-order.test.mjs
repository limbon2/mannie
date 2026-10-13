// @ts-check
import { describe, it } from 'node:test';

import { RuleTester } from 'eslint';
import * as svelteParser from 'svelte-eslint-parser';
import tseslint from 'typescript-eslint';

import { svelteScriptOrder } from '../../../scripts/eslint/svelte-script-order.mjs';

RuleTester.describe = describe;
RuleTester.it = it;
RuleTester.itOnly = it.only;

const tester = new RuleTester({
  languageOptions: { parser: svelteParser, parserOptions: { parser: tseslint.parser } },
});

/**
 * A component with the given instance script.
 * @param {string} body
 * @returns {{ code: string, filename: string }}
 */
function component(body) {
  return { code: `<script lang="ts">\n${body}\n</script>`, filename: 'Component.svelte' };
}

/**
 * @param {string} item
 * @param {string} group
 */
function moved(item, group) {
  return { data: { group, item }, messageId: 'order' };
}

tester.run('svelte-script-order', svelteScriptOrder, {
  valid: [
    component(`
      import { onMount } from 'svelte';
      interface Props { label: string }
      let { label }: Props = $props();
      const id = $props.id();
      let count = $state(0);
      let items = $state.raw<string[]>([]);
      let doubled = $derived(count * 2);
      let total = $derived.by(() => items.length);
      function increment(): void { count += 1; }
      const reset = (): void => { count = 0; };
      export function clear(): void { items = []; }
      $effect(() => { console.log(count, doubled, total, label, id); });
      onMount(() => {});
    `),
    // Other statements may stand anywhere between the props and the effects.
    component(`
      import { getContext, setContext } from 'svelte';
      let { value } = $props();
      const settings = getContext('settings');
      let theme = $state(settings.theme);
      setContext('theme', theme);
      function save(): void {}
      const saveLater = debounce(save, 300);
      $effect(() => {});
    `),
    component(`
      const LIMIT = 10;
      $effect(() => {});
    `),
    // The module script is not checked.
    {
      code: '<script lang="ts" module>export function helper() {} export type Size = "sm";</script><script lang="ts">let count = $state(0);</script>',
      filename: 'Component.svelte',
    },
    {
      code: '<script lang="ts" context="module">export function helper() {} interface Size {}</script>',
      filename: 'Component.svelte',
    },
  ],
  invalid: [
    {
      ...component(`
        function increment(): void { count += 1; }
        let count = $state(0);
      `),
      errors: [{ ...moved('$state declaration', 'functions'), line: 4 }],
    },
    {
      ...component(`
        $effect(() => {});
        function increment(): void {}
      `),
      errors: [moved('function', 'effects and lifecycle callbacks')],
    },
    {
      ...component(`
        let doubled = $derived(count * 2);
        let count = $state(0);
      `),
      errors: [moved('$state declaration', 'derived values')],
    },
    {
      ...component(`
        let count = $state(0);
        let { label } = $props();
      `),
      errors: [moved('$props declaration', 'state')],
    },
    {
      ...component(`
        let { label }: Props = $props();
        interface Props { label: string }
      `),
      errors: [moved('type declaration', 'props')],
    },
    {
      ...component(`
        onMount(() => {});
        let count = $state(0);
      `),
      errors: [moved('$state declaration', 'effects and lifecycle callbacks')],
    },
    {
      ...component(`
        $effect(() => {});
        const LIMIT = 10;
      `),
      errors: [moved('statement', 'effects and lifecycle callbacks')],
    },
    {
      ...component(`
        const LIMIT = 10;
        import { tick } from 'svelte';
      `),
      errors: [moved('import', 'other statements')],
    },
  ],
});
