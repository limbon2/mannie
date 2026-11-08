<script lang="ts">
  import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
  import { untrack } from 'svelte';

  import { Badge } from '#lib/components/ui/badge/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { type LinkFailure, linkFailureMessage } from '#lib/sdk/links.js';
  import { chatgptAccount } from '#lib/state/chatgpt-account.svelte.js';
  import { providerProfiles } from '#lib/state/provider-profiles.svelte.js';
  import { PROVIDER_SETTINGS_SECTION } from '#lib/utils/chat-urls.js';
  import { openSettings } from '#lib/utils/settings-sections.js';

  // Beside the profile menu, and above the message box of a new
  // conversation, while the profile in effect, the conversation's or the
  // default, uses the ChatGPT plan (docs/features/chatgpt-plan.md, in
  // conversations): "Using ChatGPT plan" with "Manage usage", as OpenAI's
  // guidelines ask, or "Sign in to ChatGPT" while no account is signed in,
  // which opens the Providers settings, where the account is. `profileId` is
  // the conversation's chosen profile, or `null` for the default.
  let { profileId }: { profileId: string | null } = $props();

  let linkFailure = $state<LinkFailure | null>(null);

  const effective = $derived(providerProfiles.find(profileId) ?? providerProfiles.defaultProfile);
  const usesPlan = $derived(effective?.access === 'chatgpt_plan');
  /** What the indicator shows of the account. */
  const shown = $derived.by(() => {
    if (chatgptAccount.status !== 'ready') {
      return chatgptAccount.status;
    }
    return chatgptAccount.signedIn ? 'signed_in' : 'signed_out';
  });

  async function manageUsage(): Promise<void> {
    linkFailure = await chatgptAccount.manageUsage();
  }

  function openAccount(): void {
    void openSettings(PROVIDER_SETTINGS_SECTION);
  }

  // The window follows the account once a conversation uses the plan.
  $effect(() => {
    if (usesPlan) {
      untrack(() => {
        chatgptAccount.start();
      });
    }
  });
</script>

{#if usesPlan}
  <div class="flex min-w-0 items-center gap-2" data-chatgpt-indicator={shown}>
    {#if shown === 'loading'}
      <Skeleton class="h-5 w-32" />
    {:else if shown === 'error'}
      <span class="text-sm text-destructive">{m.chat_chatgpt_indicator_failed()}</span>
      <Button size="sm" variant="link" onclick={openAccount}>{m.chat_chatgpt_open_settings()}</Button>
    {:else if shown === 'signed_in'}
      <Badge variant="secondary">{m.chat_chatgpt_using_plan()}</Badge>
      <Button size="sm" variant="link" onclick={manageUsage}>
        {m.chat_chatgpt_manage_usage()}
        <ExternalLinkIcon data-icon="inline-end" />
      </Button>
    {:else}
      <Button size="sm" variant="outline" onclick={openAccount}>{m.chat_chatgpt_indicator_sign_in()}</Button>
    {/if}
    {#if linkFailure !== null}
      <span class="text-sm text-destructive" role="alert">{linkFailureMessage(linkFailure)}</span>
    {/if}
  </div>
{/if}
