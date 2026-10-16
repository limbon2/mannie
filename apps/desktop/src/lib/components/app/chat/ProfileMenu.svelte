<script lang="ts">
  import BotIcon from '@lucide/svelte/icons/bot';
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
  import type { ConversationSummary } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatActions } from '#lib/state/chat-actions.svelte.js';
  import { providerProfiles } from '#lib/state/provider-profiles.svelte.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';
  import { PROVIDER_SETTINGS_SECTION } from '#lib/utils/chat-urls.js';
  import { openSettings } from '#lib/utils/settings-sections.js';

  // The provider profile of a conversation's next replies: one of the
  // profiles, or the default (docs/features/chat/conversation-view.md). The
  // registry's radio items keep their menu open by default; choosing a
  // profile is a single choice, so the menu closes with it.
  let { conversation }: { conversation: ConversationSummary } = $props();

  /** The menu's value for "the default profile". No profile ID can be `*`. */
  const DEFAULT_PROFILE = '*';

  let failure = $state<ChatFailure | null>(null);

  const chosen = $derived(providerProfiles.find(conversation.profileId));
  const effective = $derived(chosen ?? providerProfiles.defaultProfile);
  const effectiveName = $derived(effective?.name ?? m.chat_profile_none());
  const defaultName = $derived(providerProfiles.defaultProfile?.name ?? m.chat_profile_none());

  async function choose(value: string): Promise<void> {
    failure = await chatActions.setProfile(conversation.id, value === DEFAULT_PROFILE ? null : value);
  }
</script>

<div class="flex items-center gap-2">
  <DropdownMenu.Root>
    <DropdownMenu.Trigger>
      {#snippet child({ props })}
        <Button {...props} aria-label={m.chat_profile_trigger({ name: effectiveName })} size="sm" variant="ghost">
          <BotIcon data-icon="inline-start" />
          <span class="max-w-48 truncate" data-content>{effectiveName}</span>
          <ChevronDownIcon data-icon="inline-end" />
        </Button>
      {/snippet}
    </DropdownMenu.Trigger>
    <DropdownMenu.Content class="w-72" align="start">
      <DropdownMenu.Group>
        <DropdownMenu.Label>{m.chat_profile_menu_label()}</DropdownMenu.Label>
        <DropdownMenu.RadioGroup value={conversation.profileId ?? DEFAULT_PROFILE} onValueChange={choose}>
          <DropdownMenu.RadioItem closeOnSelect value={DEFAULT_PROFILE}>
            {m.chat_profile_default({ name: defaultName })}
          </DropdownMenu.RadioItem>
          {#each providerProfiles.profiles as profile (profile.id)}
            <DropdownMenu.RadioItem closeOnSelect value={profile.id}>
              <span data-content>{profile.name}</span>
            </DropdownMenu.RadioItem>
          {/each}
        </DropdownMenu.RadioGroup>
      </DropdownMenu.Group>
      <DropdownMenu.Separator />
      <DropdownMenu.Group>
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
