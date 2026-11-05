<script lang="ts">
  import PlugIcon from '@lucide/svelte/icons/plug';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as Empty from '#lib/components/ui/empty/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { featureName, pluginName } from '#lib/state/plugin-catalog.js';
  import { PLUGINS_SETTINGS_SECTION } from '#lib/utils/plugins-messages.js';
  import { openSettings } from '#lib/utils/settings-sections.js';

  // What an address of a plugin's view, settings section or side panel shows
  // while the plugin is off (ADR-0030, decision 11), or while the plugin's
  // optional feature that it belongs to is off (ADR-0055, decision 10): which
  // plugin and feature it belongs to, and the way to the Plugins settings,
  // where the owner turns them on.
  let { feature = null, plugin }: { feature?: string | null; plugin: string } = $props();

  const name = $derived(pluginName(plugin));
  const featureLabel = $derived(feature === null ? null : featureName(plugin, feature));
</script>

<Empty.Root class="flex-1" data-plugin-feature-off={feature ?? undefined} data-plugin-off={plugin}>
  <Empty.Header>
    <Empty.Media variant="icon">
      <PlugIcon />
    </Empty.Media>
    {#if featureLabel === null}
      <Empty.Title>{m.plugins_off_part_title({ name })}</Empty.Title>
      <Empty.Description>{m.plugins_off_part_description({ name })}</Empty.Description>
    {:else}
      <Empty.Title>{m.plugins_off_part_title({ name: featureLabel })}</Empty.Title>
      <Empty.Description>{m.plugins_off_feature_part_description({ feature: featureLabel, name })}</Empty.Description>
    {/if}
  </Empty.Header>
  <Empty.Content>
    <Button variant="outline" onclick={() => void openSettings(PLUGINS_SETTINGS_SECTION)}>
      {m.plugins_open_settings()}
    </Button>
  </Empty.Content>
</Empty.Root>
