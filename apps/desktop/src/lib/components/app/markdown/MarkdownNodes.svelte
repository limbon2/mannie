<script lang="ts">
  import SquareIcon from '@lucide/svelte/icons/square';
  import SquareCheckIcon from '@lucide/svelte/icons/square-check';

  import { Separator } from '#lib/components/ui/separator/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import type { Alignment, RenderNode } from '#lib/utils/markdown.js';

  import MarkdownCitation from './MarkdownCitation.svelte';
  import MarkdownCode from './MarkdownCode.svelte';
  import MarkdownImage from './MarkdownImage.svelte';
  import MarkdownLink from './MarkdownLink.svelte';
  import MarkdownNodes from './MarkdownNodes.svelte';

  // Every element here comes from the render tree's node kinds, and content
  // reaches the page only through text bindings: no raw HTML, and no URL in
  // an attribute (ADR-0021). Elements that hold content carry `data-content`.
  let { nodes }: { nodes: readonly RenderNode[] } = $props();

  /** A reply's headings sit below the view's own headings in the outline. */
  function headingTag(level: number): 'h3' | 'h4' | 'h5' | 'h6' {
    const levels = ['h3', 'h4', 'h5', 'h6'] as const;
    return levels[Math.min(level, levels.length) - 1] ?? 'h6';
  }

  function headingClass(level: number): string {
    if (level === 1) {
      return 'text-xl font-semibold';
    }
    return level === 2 ? 'text-lg font-semibold' : 'text-base font-semibold';
  }

  function alignClass(align: Alignment | undefined): string {
    if (align === 'center') {
      return 'text-center';
    }
    return align === 'right' ? 'text-end' : 'text-start';
  }
</script>

{#each nodes as node, index (index)}
  {#if node.kind === 'text'}
    {node.value}
  {:else if node.kind === 'paragraph'}
    <p class="my-2 first:mt-0 last:mb-0" data-content><MarkdownNodes nodes={node.children} /></p>
  {:else if node.kind === 'heading'}
    <svelte:element this={headingTag(node.level)} class="mt-4 mb-2 first:mt-0 {headingClass(node.level)}" data-content>
      <MarkdownNodes nodes={node.children} />
    </svelte:element>
  {:else if node.kind === 'emphasis'}
    <em data-content><MarkdownNodes nodes={node.children} /></em>
  {:else if node.kind === 'strong'}
    <strong class="font-semibold" data-content><MarkdownNodes nodes={node.children} /></strong>
  {:else if node.kind === 'delete'}
    <del data-content><MarkdownNodes nodes={node.children} /></del>
  {:else if node.kind === 'inline_code'}
    <code class="rounded-sm bg-muted px-1 py-0.5 font-mono text-[0.9em]" data-content>{node.value}</code>
  {:else if node.kind === 'code'}
    <MarkdownCode language={node.language} value={node.value} />
  {:else if node.kind === 'blockquote'}
    <blockquote class="my-2 border-s-2 ps-3 text-muted-foreground" data-content>
      <MarkdownNodes nodes={node.children} />
    </blockquote>
  {:else if node.kind === 'list'}
    {#if node.ordered}
      <ol class="my-2 list-decimal ps-6" start={node.start ?? undefined}>
        {#each node.items as item, itemIndex (itemIndex)}
          <li class="my-1" data-content><MarkdownNodes nodes={item.children} /></li>
        {/each}
      </ol>
    {:else}
      <ul class={['my-2 ps-6', node.items.every((item) => item.checked !== null) ? 'list-none' : 'list-disc']}>
        {#each node.items as item, itemIndex (itemIndex)}
          <li class="my-1" data-content>
            {#if item.checked !== null}
              <span class="me-1 inline-flex align-middle">
                {#if item.checked}
                  <SquareCheckIcon class="size-4" aria-hidden="true" />
                  <span class="sr-only">{m.markdown_task_done()}</span>
                {:else}
                  <SquareIcon class="size-4" aria-hidden="true" />
                  <span class="sr-only">{m.markdown_task_open()}</span>
                {/if}
              </span>
            {/if}
            <MarkdownNodes nodes={item.children} />
          </li>
        {/each}
      </ul>
    {/if}
  {:else if node.kind === 'table'}
    <!-- A fixed layout wraps long cells, so the table needs no scrolling region. -->
    <div class="my-2">
      <table class="w-full table-fixed border-collapse text-sm wrap-break-word">
        <thead>
          <tr>
            {#each node.head as cell, cellIndex (cellIndex)}
              <th class="border px-2 py-1 font-semibold {alignClass(node.align[cellIndex])}" data-content>
                <MarkdownNodes nodes={cell} />
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#each node.body as row, rowIndex (rowIndex)}
            <tr>
              {#each row as cell, cellIndex (cellIndex)}
                <td class="border px-2 py-1 {alignClass(node.align[cellIndex])}" data-content>
                  <MarkdownNodes nodes={cell} />
                </td>
              {/each}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else if node.kind === 'thematic_break'}
    <Separator class="my-3" />
  {:else if node.kind === 'break'}
    <br />
  {:else if node.kind === 'citation'}
    <MarkdownCitation key={node.key} label={node.label} provider={node.provider} />
  {:else if node.kind === 'link'}
    <MarkdownLink url={node.url}><MarkdownNodes nodes={node.children} /></MarkdownLink>
  {:else}
    <MarkdownImage description={node.description} url={node.url} />
  {/if}
{/each}
