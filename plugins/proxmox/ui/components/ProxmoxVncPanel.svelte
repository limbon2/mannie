<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Empty from '@mannie/plugin-sdk/components/empty';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import MonitorIcon from '@mannie/plugin-sdk/icons/monitor';
  import type { SidePanelProps } from '@mannie/plugin-sdk/slots';

  import { m } from '../generated/messages.js';
  import { VncSession } from '../state/vnc-session.svelte.js';
  import { proxmoxFailureMessage } from '../utils/proxmox-messages.js';
  import { parseVncPanelArgument } from '../utils/proxmox-panel.js';

  import ProxmoxVncCanvas from './ProxmoxVncCanvas.svelte';

  let { argument }: SidePanelProps = $props();

  let controlButton = $state<HTMLElement | null>(null);

  const target = $derived(parseVncPanelArgument(argument));
  const session = $derived(target === null ? null : new VncSession(target));
  const summary = $derived(session?.summary ?? null);
  const ownerInControl = $derived(summary?.control === 'owner');
  const isConnected = $derived(session?.status === 'connected');
  const interactive = $derived(isConnected && ownerInControl && session?.busy === false);

  function takeOver(): void {
    if (session !== null) {
      void session.takeOver();
    }
  }

  function resumeAgent(): void {
    if (session !== null) {
      void session.resumeAgent();
    }
  }

  function sendCtrlAltDel(): void {
    if (session !== null) {
      void session.sendCtrlAltDel();
    }
  }

  function exitConsoleFocus(): void {
    controlButton?.focus();
  }

  function closeConsole(): void {
    if (session !== null) {
      void session.closeSession();
    }
  }

  function reconnect(): void {
    if (session !== null) {
      void session.connect();
    }
  }

  $effect(() => {
    const current = session;
    if (current === null) {
      return;
    }

    void current.connect();

    return () => {
      current.stop();
    };
  });
</script>

<section
  class="flex h-full min-h-0 min-w-0 flex-col gap-2 overflow-hidden p-3"
  aria-label={m.proxmox_panel_title()}
  data-proxmox-vnc-panel=""
  data-proxmox-vnc-vmid={target?.vmid}
>
  {#if session === null}
    <Empty.Root>
      <Empty.Header>
        <Empty.Media variant="icon">
          <MonitorIcon />
        </Empty.Media>
        <Empty.Title>{m.proxmox_panel_closed_title()}</Empty.Title>
      </Empty.Header>
    </Empty.Root>
  {:else}
    <div class="flex flex-wrap items-center gap-2">
      <div class="me-auto flex min-w-0 flex-wrap items-center gap-2">
        {#if summary !== null}
          <span class="truncate text-sm font-medium" data-content dir="auto">
            {summary.vmName} ({summary.vmid})
          </span>
          <Badge data-proxmox-control-state={summary.control} variant={ownerInControl ? 'default' : 'secondary'}>
            {ownerInControl ? m.proxmox_panel_owner_control() : m.proxmox_panel_agent_control()}
          </Badge>
        {/if}
      </div>

      <div class="ms-auto flex flex-wrap items-center justify-end gap-1.5" data-proxmox-vnc-actions="">
        {#if isConnected}
          {#if ownerInControl}
            <Button
              data-proxmox-resume-agent=""
              disabled={session.busy}
              size="sm"
              bind:ref={controlButton}
              onclick={resumeAgent}
            >
              {m.proxmox_panel_resume_agent()}
            </Button>
            <Button
              data-proxmox-ctrl-alt-del=""
              disabled={session.busy}
              size="sm"
              variant="outline"
              onclick={sendCtrlAltDel}
            >
              {m.proxmox_panel_ctrl_alt_del()}
            </Button>
            <Button data-proxmox-release-focus="" size="sm" variant="outline" onclick={exitConsoleFocus}>
              {m.proxmox_panel_release_focus()}
            </Button>
          {:else}
            <Button
              data-proxmox-take-over=""
              disabled={session.busy}
              size="sm"
              bind:ref={controlButton}
              onclick={takeOver}
            >
              {m.proxmox_panel_take_over()}
            </Button>
          {/if}

          <Button
            data-proxmox-close-session=""
            disabled={session.busy}
            size="sm"
            variant="outline"
            onclick={closeConsole}
          >
            {m.proxmox_panel_close_session()}
          </Button>
        {/if}
      </div>
    </div>

    {#if session.failure !== null}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.proxmox_panel_error_title()}</Alert.Title>
        <Alert.Description>{proxmoxFailureMessage(session.failure)}</Alert.Description>
        {#if session.status === 'error'}
          <Alert.Action>
            <Button data-proxmox-reconnect="" size="sm" variant="outline" onclick={reconnect}>
              {m.proxmox_panel_reconnect()}
            </Button>
          </Alert.Action>
        {/if}
      </Alert.Root>
    {/if}

    {#if session.status === 'connecting'}
      <div class="flex flex-1 items-center justify-center gap-2 text-sm text-muted-foreground" role="status">
        <Spinner aria-hidden="true" />
        <span>{m.proxmox_panel_connecting()}</span>
      </div>
    {:else if session.status === 'closed'}
      <Empty.Root class="flex-1 border">
        <Empty.Header>
          <Empty.Media variant="icon">
            <MonitorIcon />
          </Empty.Media>
          <Empty.Title>{m.proxmox_panel_closed_title()}</Empty.Title>
          <Empty.Description>
            {m.proxmox_panel_closed_description({ reason: session.closedReason ?? 'closed' })}
          </Empty.Description>
        </Empty.Header>

        <Empty.Content>
          <Button data-proxmox-reconnect="" onclick={reconnect}>
            {m.proxmox_panel_reconnect()}
          </Button>
        </Empty.Content>
      </Empty.Root>
    {:else if isConnected}
      <ProxmoxVncCanvas
        {interactive}
        {session}
        vmName={summary?.vmName ?? String(session.target.vmid)}
        vmid={session.target.vmid}
        onExitFocus={exitConsoleFocus}
      />

      <p class="text-xs text-muted-foreground" data-proxmox-vnc-hint="" role="status">
        {ownerInControl ? m.proxmox_panel_owner_hint() : m.proxmox_panel_agent_hint()}
      </p>
    {/if}
  {/if}
</section>
