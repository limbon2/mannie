<script lang="ts">
  import KeyRoundIcon from '@lucide/svelte/icons/key-round';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { PROVIDER_SETTINGS_SECTION } from '#lib/utils/chat-urls.js';
  import { openSettings } from '#lib/utils/settings-sections.js';

  // Without a provider profile no reply can be written; the notice says so and
  // leads to the provider settings.
</script>

<Alert.Root>
  <KeyRoundIcon />
  <Alert.Title>{m.chat_no_profile_title()}</Alert.Title>
  <Alert.Description>{m.chat_no_profile_description()}</Alert.Description>
  <Alert.Action>
    <Button size="sm" variant="outline" onclick={() => void openSettings(PROVIDER_SETTINGS_SECTION)}>
      {m.chat_no_profile_action()}
    </Button>
  </Alert.Action>
</Alert.Root>
