<script lang="ts">
  import { Button } from '#lib/components/ui/button/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { PLUGINS_SETTINGS_SECTION } from '#lib/utils/plugins-messages.js';
  import { openSettings } from '#lib/utils/settings-sections.js';

  // The way to the Plugins settings beside a failed action that needs a
  // plugin that is off, such as Stop on a worker task while Workers is off
  // (ADR-0030, decision 12).
</script>

<Button
  class="h-auto self-start p-0"
  data-plugins-settings-link=""
  size="sm"
  variant="link"
  onclick={() => void openSettings(PLUGINS_SETTINGS_SECTION)}
>
  {m.plugins_open_settings()}
</Button>
