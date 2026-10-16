<script lang="ts">
  import PlugIcon from '@lucide/svelte/icons/plug';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as Empty from '#lib/components/ui/empty/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { pluginName } from '#lib/state/plugin-catalog.js';
  import { PLUGINS_SETTINGS_SECTION } from '#lib/utils/plugins-messages.js';
  import { openSettings } from '#lib/utils/settings-sections.js';

  // What an address of a plugin's view, settings section or side panel shows
  // while the plugin is off (ADR-0030, decision 11): which plugin it belongs
  // to, and the way to the Plugins settings, where the owner turns it on.
  let { plugin }: { plugin: string } = $props();

  const name = $derived(pluginName(plugin));
</script>

<Empty.Root class="flex-1" data-plugin-off={plugin}>
  <Empty.Header>
    <Empty.Media variant="icon">
      <PlugIcon />
    </Empty.Media>
    <Empty.Title>{m.plugins_off_part_title({ name })}</Empty.Title>
    <Empty.Description>{m.plugins_off_part_description({ name })}</Empty.Description>
  </Empty.Header>
  <Empty.Content>
    <Button variant="outline" onclick={() => void openSettings(PLUGINS_SETTINGS_SECTION)}>
      {m.plugins_open_settings()}
    </Button>
  </Empty.Content>
</Empty.Root>
