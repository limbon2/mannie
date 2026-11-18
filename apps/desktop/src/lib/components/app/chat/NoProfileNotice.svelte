<script lang="ts">
  import KeyRoundIcon from '@lucide/svelte/icons/key-round';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import type { ProviderProfile } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatgptAccount } from '#lib/state/chatgpt-account.svelte.js';
  import { providerProfiles } from '#lib/state/provider-profiles.svelte.js';
  import { chatgptFailureMessage, profileFailureMessage } from '#lib/utils/chat-errors.js';
  import { PROVIDER_SETTINGS_SECTION } from '#lib/utils/chat-urls.js';
  import { openSettings } from '#lib/utils/settings-sections.js';

  interface Props {
    profile?: ProviderProfile;
  }

  let { profile }: Props = $props();

  const loading = $derived(
    providerProfiles.status === 'loading' ||
      providerProfiles.catalogStatus === 'loading' ||
      chatgptAccount.status === 'loading',
  );
  const canRetry = $derived(
    chatgptAccount.signedIn || chatgptAccount.failure !== null || providerProfiles.failure !== null,
  );
  const reason = $derived.by(() => {
    if (providerProfiles.failure !== null) {
      return profileFailureMessage(providerProfiles.failure);
    }
    if (chatgptAccount.failure !== null) {
      return chatgptFailureMessage(chatgptAccount.failure);
    }
    if (providerProfiles.catalogFailure !== null) {
      return m.chat_chatgpt_models_failed({ reason: chatgptFailureMessage(providerProfiles.catalogFailure) });
    }
    if (profile?.catalog) {
      return m.chat_profile_unavailable({ name: profile.name });
    }
    return chatgptAccount.signedIn ? m.chat_chatgpt_models_empty() : m.chat_no_profile_description();
  });

  function retry(): void {
    if (providerProfiles.failure !== null) {
      void providerProfiles.reload();
    } else if (chatgptAccount.failure !== null) {
      chatgptAccount.retry();
    } else {
      void providerProfiles.refreshCatalog();
    }
  }
</script>

{#if loading}
  <p class="text-sm text-muted-foreground" role="status">{m.chat_loading()}</p>
{:else}
  <Alert.Root>
    <KeyRoundIcon />
    <Alert.Title>{m.chat_no_profile_title()}</Alert.Title>
    <Alert.Description>
      {reason}
      <div class="flex flex-wrap gap-2 pt-1">
        {#if canRetry}
          <Button size="sm" variant="outline" onclick={retry}>{m.chat_try_again()}</Button>
        {/if}
        <Button size="sm" variant="outline" onclick={() => void openSettings(PROVIDER_SETTINGS_SECTION)}>
          {m.chat_no_profile_action()}
        </Button>
      </div>
    </Alert.Description>
  </Alert.Root>
{/if}
