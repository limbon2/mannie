<script lang="ts">
  import * as DropdownMenu from '@mannie/plugin-sdk/components/dropdown-menu';
  import MonitorIcon from '@mannie/plugin-sdk/icons/monitor';
  import ServerIcon from '@mannie/plugin-sdk/icons/server';
  import { call } from '@mannie/plugin-sdk/ipc';
  import { openSettings } from '@mannie/plugin-sdk/navigation';
  import type { ConversationControlProps } from '@mannie/plugin-sdk/slots';

  import { commands, type ServerView, type VncSessionSummary } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { proxmoxActions, SETTINGS_SECTION } from '../state/proxmox-actions.svelte.js';
  import { serverList } from '../state/servers.svelte.js';

  interface AllowedVmMenuItem {
    readonly server: ServerView;
    readonly vmid: number;
    readonly vmName: string;
  }

  let { conversation, opener }: ConversationControlProps = $props();

  let sessions = $state<VncSessionSummary[]>([]);

  let generation = 0;

  const allowedEntries = $derived.by(() => {
    const items: AllowedVmMenuItem[] = [];
    for (const server of serverList.servers) {
      if (!server.passwordSet) {
        continue;
      }
      for (const vm of server.allowedVms) {
        items.push({ server, vmid: vm.vmid, vmName: vm.name });
      }
    }
    return items;
  });

  async function refreshSessions(): Promise<void> {
    const conversationId = conversation.id;
    const request = ++generation;
    const outcome = await call(() => commands.listVncSessions({ conversationId }));
    const isStale = request !== generation;
    const isDifferentConversation = conversationId !== conversation.id;
    if (isStale || isDifferentConversation) {
      return;
    }

    if (outcome.ok) {
      sessions = outcome.data;
    }
  }

  function handleMenuOpenChange(open: boolean): void {
    if (open) {
      void refreshSessions();
    }
  }

  function openConsole(serverId: string, vmid: number, vmName: string): void {
    void proxmoxActions.openConsole(conversation.id, serverId, vmid, vmName, opener);
  }

  function openProxmoxSettings(): void {
    void openSettings(SETTINGS_SECTION);
  }
</script>

<DropdownMenu.Sub onOpenChange={handleMenuOpenChange}>
  <DropdownMenu.SubTrigger data-proxmox-control="">
    <MonitorIcon />
    <span class="flex-1">{m.proxmox_control_label()}</span>
  </DropdownMenu.SubTrigger>

  <DropdownMenu.SubContent class="w-72">
    {#if sessions.length > 0}
      <DropdownMenu.Group>
        <DropdownMenu.GroupHeading>{m.proxmox_control_open_sessions()}</DropdownMenu.GroupHeading>
        {#each sessions as session (`${session.serverId}:${String(session.vmid)}`)}
          <DropdownMenu.Item
            data-proxmox-open-session={`${session.serverId}:${String(session.vmid)}`}
            textValue={`${session.vmName} (${String(session.vmid)}) ${session.serverName}`}
            onSelect={() => openConsole(session.serverId, session.vmid, session.vmName)}
          >
            <MonitorIcon />
            <span class="truncate" data-content dir="auto">
              {session.vmName} ({session.vmid}) · {session.serverName}
            </span>
          </DropdownMenu.Item>
        {/each}
      </DropdownMenu.Group>
      <DropdownMenu.Separator />
    {/if}

    <DropdownMenu.Group>
      <DropdownMenu.GroupHeading>{m.proxmox_control_allowed_vms()}</DropdownMenu.GroupHeading>
      {#if allowedEntries.length === 0}
        <DropdownMenu.Item disabled>{m.proxmox_control_empty()}</DropdownMenu.Item>
      {:else}
        {#each allowedEntries as entry (`${entry.server.id}:${String(entry.vmid)}`)}
          <DropdownMenu.Item
            data-proxmox-open-vm={`${entry.server.id}:${String(entry.vmid)}`}
            textValue={`${entry.vmName} (${String(entry.vmid)}) ${entry.server.name}`}
            onSelect={() => openConsole(entry.server.id, entry.vmid, entry.vmName)}
          >
            <ServerIcon />
            <span class="truncate" data-content dir="auto">
              {entry.vmName} ({entry.vmid}) · {entry.server.name}
            </span>
          </DropdownMenu.Item>
        {/each}
      {/if}
    </DropdownMenu.Group>

    <DropdownMenu.Separator />

    <DropdownMenu.Group>
      <DropdownMenu.Item data-proxmox-open-settings="" onSelect={openProxmoxSettings}>
        {m.proxmox_control_open_settings()}
      </DropdownMenu.Item>
    </DropdownMenu.Group>
  </DropdownMenu.SubContent>
</DropdownMenu.Sub>
