<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import MonitorIcon from '@mannie/plugin-sdk/icons/monitor';
  import type { ToolKindDetailsProps } from '@mannie/plugin-sdk/slots';

  import { m } from '../generated/messages.js';
  import { serverList } from '../state/servers.svelte.js';
  import { openVncPanel } from '../utils/proxmox-panel.js';

  let { call, conversationId }: ToolKindDetailsProps = $props();

  let trigger = $state<HTMLElement | null>(null);

  const serverName = $derived(readStringField(call.data, 'server'));
  const node = $derived(readStringField(call.data, 'node'));
  const vmid = $derived(readNumberField(call.data, 'vmid'));
  const vmName = $derived(readStringField(call.data, 'vmName'));
  const device = $derived(readStringField(call.data, 'device'));
  const volid = $derived(readStringField(call.data, 'volid'));
  const width = $derived(readNumberField(call.data, 'width'));
  const height = $derived(readNumberField(call.data, 'height'));
  const action = $derived(readStringField(call.data, 'action'));
  const isVncAction = $derived(action === 'vnc_screenshot' || action === 'vnc_pointer' || action === 'vnc_keyboard');
  const matchingServer = $derived(
    serverName === null ? undefined : serverList.servers.find((server) => server.name === serverName),
  );
  const hasFrameSize = $derived(width !== null && height !== null);
  const hasDetails = $derived(
    serverName !== null || vmid !== null || node !== null || device !== null || volid !== null || hasFrameSize,
  );
  const canOpenConsole = $derived(isVncAction && matchingServer !== undefined && vmid !== null);

  function readStringField(data: Readonly<Record<string, unknown>> | null, key: string): string | null {
    const value = data?.[key];
    if (typeof value !== 'string') {
      return null;
    }

    return value === '' ? null : value;
  }

  function readNumberField(data: Readonly<Record<string, unknown>> | null, key: string): number | null {
    const value = data?.[key];
    if (typeof value !== 'number') {
      return null;
    }

    return Number.isFinite(value) ? value : null;
  }

  function openLiveConsole(): void {
    if (matchingServer === undefined) {
      return;
    }

    if (vmid === null) {
      return;
    }

    void openVncPanel({ conversationId, serverId: matchingServer.id, vmid }, vmName ?? String(vmid), {
      opener: trigger,
    });
  }
</script>

{#if hasDetails}
  <div class="flex flex-col gap-2">
    <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
      {#if serverName !== null}
        <dt class="text-muted-foreground">{m.proxmox_tool_detail_server()}</dt>
        <dd class="wrap-anywhere" data-content dir="auto">{serverName}</dd>
      {/if}

      {#if vmid !== null}
        <dt class="text-muted-foreground">{m.proxmox_tool_detail_vm()}</dt>
        <dd class="font-mono wrap-anywhere" data-content dir="ltr">
          {vmName === null ? String(vmid) : `${vmName} (${String(vmid)})`}
        </dd>
      {/if}

      {#if node !== null}
        <dt class="text-muted-foreground">{m.proxmox_tool_detail_node()}</dt>
        <dd class="font-mono wrap-anywhere" data-content dir="ltr">{node}</dd>
      {/if}

      {#if device !== null}
        <dt class="text-muted-foreground">{m.proxmox_tool_detail_device()}</dt>
        <dd class="font-mono wrap-anywhere" data-content dir="ltr">{device}</dd>
      {/if}

      {#if volid !== null}
        <dt class="text-muted-foreground">{m.proxmox_tool_detail_iso()}</dt>
        <dd class="font-mono wrap-anywhere" data-content dir="ltr">{volid}</dd>
      {/if}

      {#if hasFrameSize}
        <dt class="text-muted-foreground">{m.proxmox_tool_detail_frame()}</dt>
        <dd class="font-mono" data-content dir="ltr">
          {m.proxmox_tool_detail_frame_size({ height: String(height), width: String(width) })}
        </dd>
      {/if}
    </dl>

    {#if canOpenConsole}
      <Button class="self-start" size="sm" variant="outline" bind:ref={trigger} onclick={openLiveConsole}>
        <MonitorIcon data-icon="inline-start" />
        {m.proxmox_panel_title()}
      </Button>
    {/if}
  </div>
{/if}
