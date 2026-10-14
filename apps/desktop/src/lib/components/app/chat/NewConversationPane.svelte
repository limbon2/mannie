<script lang="ts">
  import MessagesSquareIcon from '@lucide/svelte/icons/messages-square';

  import * as Empty from '#lib/components/ui/empty/index.js';
  import { commands } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { providerProfiles } from '#lib/state/provider-profiles.svelte.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import { call } from '#lib/utils/calls.js';
  import { openConversation } from '#lib/utils/chat-urls.js';

  import Composer from './Composer.svelte';
  import NoProfileNotice from './NoProfileNotice.svelte';

  // A new conversation: the message box, with a hint about what the owner can
  // do. The first message creates the conversation, which then opens.
  const noProfile = $derived(providerProfiles.status === 'ready' && providerProfiles.profiles.length === 0);

  async function start(text: string): Promise<ChatFailure | null> {
    const outcome = await call(() => commands.startConversation({ profileId: null, text }));
    if (!outcome.ok) {
      return outcome.failure;
    }
    await openConversation(outcome.data.conversationId);
    return null;
  }
</script>

<div class="flex min-h-0 flex-1 flex-col">
  <div class="flex min-h-0 flex-1 flex-col items-center justify-center gap-4 overflow-y-auto p-6">
    {#if noProfile}
      <div class="w-full max-w-xl">
        <NoProfileNotice />
      </div>
    {:else}
      <Empty.Root>
        <Empty.Header>
          <Empty.Media variant="icon">
            <MessagesSquareIcon />
          </Empty.Media>
          <Empty.Title>{m.chat_new_title()}</Empty.Title>
          <Empty.Description>{m.chat_new_description()}</Empty.Description>
        </Empty.Header>
      </Empty.Root>
    {/if}
  </div>
  <Composer disabled={noProfile} onSend={start} />
</div>
