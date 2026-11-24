<script lang="ts">
  import BotIcon from '@lucide/svelte/icons/bot';
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';

  import { Badge } from '#lib/components/ui/badge/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { providerProfiles } from '#lib/state/provider-profiles.svelte.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';
  import { PROVIDER_SETTINGS_SECTION } from '#lib/utils/chat-urls.js';
  import { planCatalogMessage } from '#lib/utils/provider-labels.js';
  import { openSettings } from '#lib/utils/settings-sections.js';

  // The provider profile of a conversation's next replies, or of a new
  // conversation's first: one of the profiles, or the default
  // (docs/features/chat/conversation-view.md). `profileId` is the chosen
  // profile, or `null` for the default, and `onChoose` applies a choice. The
  // default profile is marked inline inside its group instead of duplicated at
  // the top of the menu; choosing it clears the per-conversation override.
  interface Props {
    onChoose: (profileId: string | null) => Promise<ChatFailure | null>;
    profileId: string | null;
  }

  let { onChoose, profileId }: Props = $props();

  let failure = $state<ChatFailure | null>(null);

  const chosen = $derived(providerProfiles.find(profileId));
  const defaultProfileId = $derived(providerProfiles.defaultProfile?.id ?? null);
  const effective = $derived(chosen ?? providerProfiles.defaultProfile);
  const effectiveName = $derived(effective?.name ?? m.chat_profile_none());
  const showsPlan = $derived(providerProfiles.planModels.length > 0 || chosen?.catalog !== undefined);
  const hasConfigured = $derived(providerProfiles.configured.length > 0);
  const hasProfiles = $derived(showsPlan || hasConfigured);
  const catalogMessage = $derived(planCatalogMessage(providerProfiles));

  async function choose(value: string): Promise<void> {
    failure = await onChoose(value === defaultProfileId ? null : value);
  }
</script>

<div class="flex items-center gap-2">
  <DropdownMenu.Root>
    <DropdownMenu.Trigger>
      {#snippet child({ props })}
        <Button
          {...props}
          aria-label={m.chat_profile_trigger({ name: effectiveName })}
          data-model-picker=""
          size="sm"
          variant="ghost"
        >
          <BotIcon data-icon="inline-start" />
          <span class="max-w-48 truncate" data-content>{effectiveName}</span>
          <ChevronDownIcon data-icon="inline-end" />
        </Button>
      {/snippet}
    </DropdownMenu.Trigger>

    <DropdownMenu.Content class="w-72" align="start">
      {#if hasProfiles}
        <DropdownMenu.RadioGroup value={effective?.id ?? ''} onValueChange={choose}>
          {#if showsPlan}
            <DropdownMenu.Group>
              <DropdownMenu.Label>{m.chat_chatgpt_plan()}</DropdownMenu.Label>
              {#each providerProfiles.planModels as profile (profile.id)}
                <DropdownMenu.RadioItem closeOnSelect data-plan-model={profile.model} value={profile.id}>
                  <span class="min-w-0 truncate" data-content>{profile.name}</span>
                  {#if profile.id === defaultProfileId}
                    <Badge class="ms-auto shrink-0" variant="secondary">{m.chat_providers_default()}</Badge>
                  {/if}
                </DropdownMenu.RadioItem>
              {/each}

              {#if chosen?.catalog?.available === false}
                <DropdownMenu.RadioItem disabled value={chosen.id}>
                  <span class="min-w-0 truncate" data-content>{chosen.name}</span>
                  {#if chosen.id === defaultProfileId}
                    <Badge class="ms-auto shrink-0" variant="secondary">{m.chat_providers_default()}</Badge>
                  {/if}
                </DropdownMenu.RadioItem>
              {/if}
            </DropdownMenu.Group>
          {/if}

          {#if hasConfigured}
            <DropdownMenu.Group>
              <DropdownMenu.Label>{m.chat_profile_configured()}</DropdownMenu.Label>
              {#each providerProfiles.configured as profile (profile.id)}
                <DropdownMenu.RadioItem closeOnSelect value={profile.id}>
                  <span class="min-w-0 truncate" data-content>{profile.name}</span>
                  {#if profile.id === defaultProfileId}
                    <Badge class="ms-auto shrink-0" variant="secondary">{m.chat_providers_default()}</Badge>
                  {/if}
                </DropdownMenu.RadioItem>
              {/each}
            </DropdownMenu.Group>
          {/if}
        </DropdownMenu.RadioGroup>
        <DropdownMenu.Separator />
      {/if}

      <DropdownMenu.Group>
        {#if catalogMessage !== null}
          <DropdownMenu.Label>{catalogMessage}</DropdownMenu.Label>
          {#if providerProfiles.catalogStatus !== 'loading'}
            <DropdownMenu.Item onSelect={() => void providerProfiles.refreshCatalog()}>
              {m.chat_try_again()}
            </DropdownMenu.Item>
          {/if}
        {/if}
        <DropdownMenu.Item onSelect={() => void openSettings(PROVIDER_SETTINGS_SECTION)}>
          {m.chat_profile_manage()}
        </DropdownMenu.Item>
      </DropdownMenu.Group>
    </DropdownMenu.Content>
  </DropdownMenu.Root>

  {#if failure !== null}
    <span class="text-sm text-destructive" role="alert">{chatFailureMessage(failure)}</span>
  {/if}
</div>
