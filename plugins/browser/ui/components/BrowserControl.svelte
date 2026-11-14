<script lang="ts">
  import * as AlertDialog from '@mannie/plugin-sdk/components/alert-dialog';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as DropdownMenu from '@mannie/plugin-sdk/components/dropdown-menu';
  import GlobeIcon from '@mannie/plugin-sdk/icons/globe';
  import { call } from '@mannie/plugin-sdk/ipc';
  import type { ConversationControlProps } from '@mannie/plugin-sdk/slots';

  import { type BrowserControl, type BrowserStatus, commands } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { browserFailureMessage } from '../utils/browser-messages.js';
  import { browserTabLabel, openBrowser, openBrowserTab } from '../utils/browser-panel.js';

  let { conversation }: ConversationControlProps = $props();
  let trigger = $state<HTMLElement | null>(null);
  let checking = $state(false);
  let opening = $state(false);
  let failure = $state<string | null>(null);
  let loadFailure = $state<string | null>(null);
  let status = $state.raw<BrowserStatus | null>(null);
  let generation = 0;
  const tabs = $derived(status?.tabs ?? []);

  async function loadTabs(): Promise<void> {
    const conversationId = conversation.id;
    const request = ++generation;
    checking = true;
    loadFailure = null;
    status = null;
    const outcome = await call(() => commands.browserStatus({ conversationId }));
    const isLatest = request === generation;
    const sameConversation = conversationId === conversation.id;
    if (isLatest && sameConversation) {
      if (outcome.ok) {
        status = outcome.data;
      } else {
        loadFailure = browserFailureMessage(outcome.failure);
      }
      checking = false;
    }
  }

  async function openNew(): Promise<void> {
    const currentStatus = status;
    const unknownStatus = currentStatus === null;
    if (unknownStatus || opening) {
      return;
    }
    const conversationId = conversation.id;
    const action: BrowserControl = currentStatus.control === null ? { kind: 'start' } : { kind: 'open_tab' };
    failure = null;
    opening = true;
    try {
      const outcome = await call(() => commands.browserControl({ action, conversationId, tabId: null }));
      if (outcome.ok) {
        await openBrowser(conversationId, trigger);
      } else if (conversationId === conversation.id) {
        failure = browserFailureMessage(outcome.failure);
      }
    } finally {
      opening = false;
    }
  }
</script>

<DropdownMenu.Root
  onOpenChange={(open) => {
    if (open) {
      void loadTabs();
    }
  }}
>
  <DropdownMenu.Trigger>
    {#snippet child({ props })}
      <Button {...props} data-browser-control="" disabled={opening} size="sm" variant="ghost" bind:ref={trigger}>
        <GlobeIcon data-icon="inline-start" />
        <span>{m.browser_panel_title()}</span>
      </Button>
    {/snippet}
  </DropdownMenu.Trigger>
  <DropdownMenu.Content class="w-72" align="end">
    {#if checking}
      <DropdownMenu.Group>
        <DropdownMenu.Item disabled>{m.browser_loading()}</DropdownMenu.Item>
      </DropdownMenu.Group>
    {:else}
      {#if tabs.length > 0}
        <DropdownMenu.Group>
          <DropdownMenu.Label>{m.browser_tabs()}</DropdownMenu.Label>
          {#each tabs as tab (tab.id)}
            {const label = $derived(browserTabLabel(tab))}
            <DropdownMenu.Item
              data-browser-open-tab=""
              onSelect={() =>
                void openBrowserTab(conversation.id, tab, { opener: trigger, replaceArgument: conversation.id })}
            >
              <GlobeIcon />
              <span class="truncate" data-content={label.isContent ? '' : undefined} dir="auto">{label.text}</span>
            </DropdownMenu.Item>
          {/each}
        </DropdownMenu.Group>
        <DropdownMenu.Separator />
      {/if}
      <DropdownMenu.Group>
        {#if status === null}
          {#if loadFailure !== null}<DropdownMenu.Item disabled>{loadFailure}</DropdownMenu.Item>{/if}
          <DropdownMenu.Item
            onSelect={(event: Event) => {
              event.preventDefault();
              void loadTabs();
            }}
          >
            {m.browser_retry()}
          </DropdownMenu.Item>
        {:else}
          <DropdownMenu.Item
            data-browser-new-tab=""
            disabled={status.control === 'agent'}
            onSelect={() => void openNew()}
          >
            {m.browser_new_tab()}
          </DropdownMenu.Item>
        {/if}
      </DropdownMenu.Group>
    {/if}
  </DropdownMenu.Content>
</DropdownMenu.Root>

<AlertDialog.Root
  bind:open={
    () => failure !== null,
    (open) => {
      if (!open) {
        failure = null;
      }
    }
  }
>
  <AlertDialog.Content>
    <AlertDialog.Header>
      <AlertDialog.Title>{m.browser_error_title()}</AlertDialog.Title>
      <AlertDialog.Description>{failure ?? ''}</AlertDialog.Description>
    </AlertDialog.Header>
    <AlertDialog.Footer>
      <AlertDialog.Cancel>{m.browser_cancel()}</AlertDialog.Cancel>
    </AlertDialog.Footer>
  </AlertDialog.Content>
</AlertDialog.Root>
