<script lang="ts">
  import { tokenizeUrlTemplate, type UrlTokenKind } from '../utils/network-urls.js';

  interface Props {
    class?: string;
    value: string;
  }

  let { class: className = '', value }: Props = $props();

  const tokens = $derived(tokenizeUrlTemplate(value));

  function tokenClass(kind: UrlTokenKind): string {
    switch (kind) {
      case 'variable': {
        return 'rounded-[3px] bg-token-variable px-0.5 font-medium text-token-variable-foreground ring-1 ring-token-variable-border/55 ring-inset';
      }
      case 'path': {
        return 'rounded-[3px] bg-token-path px-0.5 font-medium text-token-path-foreground ring-1 ring-token-path-border/55 ring-inset';
      }
      case 'query_punct': {
        return 'font-semibold text-token-query-foreground';
      }
      case 'query_key': {
        return 'rounded-[3px] bg-token-query px-0.5 font-medium text-token-query-foreground ring-1 ring-token-query-border/55 ring-inset';
      }
      case 'query_value': {
        return 'text-token-query-foreground';
      }
      case 'text': {
        return '';
      }
    }
  }
</script>

<span class="font-mono {className}" data-content="" dir="ltr">
  {#each tokens as token, idx (idx)}
    {#if token.kind === 'text'}
      {token.text}
    {:else}
      <span class={tokenClass(token.kind)} data-network-token={token.kind}>{token.text}</span>
    {/if}
  {/each}
</span>
