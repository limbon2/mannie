<script lang="ts">
  import LayoutDashboardIcon from '@lucide/svelte/icons/layout-dashboard';

  import { page } from '$app/state';

  import * as Empty from '#lib/components/ui/empty/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import { activeView } from '#lib/utils/views.js';

  import PluginOffState from './PluginOffState.svelte';

  const slots = getShellSlots();

  const active = $derived(activeView(slots.views, page.url));
</script>

{#if active.kind === 'view'}
  <active.view.component />
{:else if active.kind === 'plugin_off'}
  <PluginOffState plugin={active.plugin} />
{:else}
  <Empty.Root class="flex-1">
    <Empty.Header>
      <Empty.Media variant="icon">
        <LayoutDashboardIcon />
      </Empty.Media>
      {#if active.kind === 'missing'}
        <Empty.Title>{m.shell_view_missing_title()}</Empty.Title>
        <Empty.Description>{m.shell_view_missing_description({ view: active.requested })}</Empty.Description>
      {:else}
        <Empty.Title>{m.shell_main_empty_title()}</Empty.Title>
        <Empty.Description>{m.shell_main_empty_description()}</Empty.Description>
      {/if}
    </Empty.Header>
  </Empty.Root>
{/if}
