<script lang="ts">
  import PanelLeftIcon from '@lucide/svelte/icons/panel-left';

  import { Button } from '#lib/components/ui/button/index.js';
  import { Kbd } from '#lib/components/ui/kbd/index.js';
  import { useSidebar } from '#lib/components/ui/sidebar/index.js';
  import * as Tooltip from '#lib/components/ui/tooltip/index.js';
  import { m } from '#lib/paraglide/messages.js';

  const sidebar = useSidebar();
</script>

<Tooltip.Root>
  <Tooltip.Trigger>
    {#snippet child({ props })}
      <Button
        {...props}
        aria-label={m.shell_sidebar_show_hide()}
        size="icon-sm"
        variant="ghost"
        onclick={sidebar.toggle}
      >
        <PanelLeftIcon />
      </Button>
    {/snippet}
  </Tooltip.Trigger>
  <Tooltip.Content>
    <span>{m.shell_sidebar_show_hide()}</span>
    <Kbd>{m.shell_shortcut_show_hide_sidebar()}</Kbd>
  </Tooltip.Content>
</Tooltip.Root>
