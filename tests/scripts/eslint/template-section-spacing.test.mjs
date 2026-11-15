// @ts-check
import { describe, it } from 'node:test';

import { RuleTester } from 'eslint';
import * as svelteParser from 'svelte-eslint-parser';

import { templateSectionSpacing } from '../../../scripts/eslint/template-section-spacing.mjs';

RuleTester.describe = describe;
RuleTester.it = it;
RuleTester.itOnly = it.only;

const tester = new RuleTester({ languageOptions: { parser: svelteParser } });
const filename = 'Component.svelte';
const blankLine = { messageId: 'blankLine' };
const adjacentConditions = `{#if pending}
  <Loading />
{/if}
{#if failed}
  <Failure />
{/if}`;

tester.run('mannie/template-section-spacing', templateSectionSpacing, {
  valid: [
    {
      filename,
      code: `<section>
  {#if pending}
    <Pending />
  {/if}

  {#if failed}
    <Failure />
  {/if}
</section>`,
    },
    {
      filename,
      code: `<ul>
  <li>
    <First />
  </li>
  <li>
    <Second />
  </li>
</ul>`,
    },
    {
      filename,
      code: `<div>
  <span>First</span>
  <span>Second</span>
</div>`,
    },
    {
      filename,
      code: `<section>
  <div>
    <Details />
  </div>
  <Button
    disabled={saving}
    onclick={save}>{label}
  </Button>
</section>`,
    },
    {
      filename,
      code: `<div>
  <First />
</div><div>
  <Second />
</div>`,
    },
    {
      filename,
      code: `<p>
  Showing
  {#if pending}
    <em>pending</em>
  {/if}
  {#if failed}
    <strong>failed</strong>
  {/if}
  requests.
</p>`,
    },
    {
      filename,
      code: `<pre>
  {#if first}
    <code>first</code>
  {/if}
  {#if second}
    <code>second</code>
  {/if}
</pre>`,
    },
    {
      filename,
      code: `<svelte:options preserveWhitespace />\n${adjacentConditions}`,
    },
    {
      filename,
      code: `<svelte:options preserveWhitespace={true} />\n${adjacentConditions}`,
    },
    {
      filename,
      code: `{#if pending}
  <Loading />
{:else}
  <Results />
{/if}`,
    },
  ],
  invalid: [
    {
      filename,
      code: `<Dialog.Content>
  {#if pending}
    <Loading />
  {/if}
  {#if failed}
    <Failure />
  {/if}

  <div>
    <Details />
  </div>
  <div>
    <Options />
  </div>

  {#if warning}
    <Warning />
  {/if}
  <Dialog.Footer>
    <Save />
  </Dialog.Footer>
</Dialog.Content>`,
      output: `<Dialog.Content>
  {#if pending}
    <Loading />
  {/if}

  {#if failed}
    <Failure />
  {/if}

  <div>
    <Details />
  </div>

  <div>
    <Options />
  </div>

  {#if warning}
    <Warning />
  {/if}

  <Dialog.Footer>
    <Save />
  </Dialog.Footer>
</Dialog.Content>`,
      errors: [blankLine, blankLine, blankLine],
    },
    {
      filename,
      code: '<section>\r\n  <Content />\r\n</section>\r\n{#if ready}\r\n  <Actions />\r\n{/if}',
      output: '<section>\r\n  <Content />\r\n</section>\r\n\r\n{#if ready}\r\n  <Actions />\r\n{/if}',
      errors: [blankLine],
    },
    {
      filename,
      code: `{#await request}
  <Loading />
{:then results}
  {#if results.length}
    <Summary {results} />
  {/if}
  <div>
    <Results {results} />
  </div>
{:catch error}
  <Failure {error} />
{/await}`,
      output: `{#await request}
  <Loading />
{:then results}
  {#if results.length}
    <Summary {results} />
  {/if}

  <div>
    <Results {results} />
  </div>
{:catch error}
  <Failure {error} />
{/await}`,
      errors: [blankLine],
    },
  ],
});
