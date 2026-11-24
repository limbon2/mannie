<script lang="ts">
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { formatBytes, formatDuration } from '@mannie/plugin-sdk/format';
  import type { ToolKindDetailsProps } from '@mannie/plugin-sdk/slots';

  import { m } from '../generated/messages.js';
  import { readSendCardDetails } from '../utils/network-tools.js';

  import MethodBadge from './MethodBadge.svelte';

  let { call }: ToolKindDetailsProps = $props();

  const details = $derived(readSendCardDetails(call.data));
</script>

{#if details !== null}
  <div class="mt-2 flex flex-col gap-1.5 rounded-md border border-border bg-muted/30 p-2 text-xs">
    <div class="flex flex-wrap items-center gap-2">
      {#if details.protocol !== null}
        <Badge class="font-mono text-[10px] uppercase" variant="secondary">
          {details.protocol === 'http' ? m.network_protocol_http() : details.protocol}
        </Badge>
      {/if}

      {#if details.method !== null}
        <MethodBadge method={details.method} />
      {/if}

      {#if details.status !== null}
        <Badge variant={details.status >= 400 ? 'destructive' : 'outline'}>
          {m.network_tool_details_status({
            status: details.statusText ? `${details.status} ${details.statusText}` : String(details.status),
          })}
        </Badge>
      {/if}

      {#if details.durationMs !== null}
        <span class="text-muted-foreground" data-content="">
          {formatDuration(details.durationMs)}
        </span>
      {/if}

      {#if details.bytes !== null}
        <span class="text-muted-foreground" data-content="">
          {formatBytes(details.bytes)}
        </span>
      {/if}

      {#if details.truncated}
        <Badge variant="secondary">{m.network_tool_details_truncated()}</Badge>
      {/if}
    </div>

    {#if details.url !== null}
      <p class="truncate font-mono text-[11px] text-muted-foreground" data-content="" dir="ltr">
        {details.url}
      </p>
    {/if}
  </div>
{/if}
