<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as DropdownMenu from '@mannie/plugin-sdk/components/dropdown-menu';
  import ChevronDownIcon from '@mannie/plugin-sdk/icons/chevron-down';
  import GlobeIcon from '@mannie/plugin-sdk/icons/globe';
  import type { ConversationControlProps } from '@mannie/plugin-sdk/slots';

  import type { BrowserTab } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { browserTabLabel, openBrowser, openBrowserTab, readBrowserStatus } from '../utils/browser-panel.js';

  let { conversation }: ConversationControlProps = $props();
  let trigger = $state<HTMLElement | null>(null);
  let chooser = $state<HTMLElement | null>(null);
  let checking = $state(false);
  let tabs = $state.raw<BrowserTab[]>([]);

  async function loadTabs(): Promise<void> {
    const conversationId = conversation.id;
    checking = true;
    tabs = [];
    const status = await readBrowserStatus(conversationId);
    if (conversationId === conversation.id) {
      tabs = status?.tabs ?? [];
      checking = false;
    }
  }
</script>

<div class="flex items-center">
  <Button
    data-browser-control=""
    size="sm"
    variant="ghost"
    bind:ref={trigger}
    onclick={() => void openBrowser(conversation.id, trigger)}
  >
    <GlobeIcon data-icon="inline-start" />
    {m.browser_panel_title()}
  </Button>
  <DropdownMenu.Root
    onOpenChange={(open) => {
      if (open) {
        void loadTabs();
      }
    }}
  >
    <DropdownMenu.Trigger aria-label={m.browser_tabs()} bind:ref={chooser}>
      {#snippet child({ props })}
        <Button {...props} size="icon-sm" title={m.browser_tabs()} variant="ghost">
          <ChevronDownIcon />
        </Button>
      {/snippet}
    </DropdownMenu.Trigger>
    <DropdownMenu.Content class="max-w-64" align="start">
      <DropdownMenu.Group>
        {#if checking}
          <DropdownMenu.Item disabled>{m.browser_loading()}</DropdownMenu.Item>
        {:else if tabs.length === 0}
          <DropdownMenu.Item disabled>{m.browser_no_tab()}</DropdownMenu.Item>
        {:else}
          {#each tabs as tab (tab.id)}
            {const label = $derived(browserTabLabel(tab))}
            <DropdownMenu.Item
              onSelect={() =>
                void openBrowserTab(conversation.id, tab, { opener: chooser, replaceArgument: conversation.id })}
            >
              <span class="truncate" data-content={label.isContent ? '' : undefined} dir="auto">
                {label.text}
              </span>
            </DropdownMenu.Item>
          {/each}
        {/if}
      </DropdownMenu.Group>
    </DropdownMenu.Content>
  </DropdownMenu.Root>
</div>
