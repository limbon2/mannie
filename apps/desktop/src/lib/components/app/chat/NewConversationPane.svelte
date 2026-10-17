<script lang="ts">
  import MessagesSquareIcon from '@lucide/svelte/icons/messages-square';

  import * as Empty from '#lib/components/ui/empty/index.js';
  import { commands, type MentionRef } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { conversationList } from '#lib/state/conversation-list.svelte.js';
  import { providerProfiles } from '#lib/state/provider-profiles.svelte.js';
  import { markStartedConversation } from '#lib/state/started-conversation.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import { call } from '#lib/utils/calls.js';
  import { newConversationProjectId, openConversation } from '#lib/utils/chat-urls.js';

  import Composer from './Composer.svelte';
  import NoProfileNotice from './NoProfileNotice.svelte';

  // A new conversation: the message box, with a hint about what the owner can
  // do. The first message creates the conversation, which then opens. A new
  // conversation of a project, which the address names, starts in it and
  // works in its folder (docs/features/chat/projects.md); the heading names
  // the project, and nothing changes near the message box.
  const noProfile = $derived(providerProfiles.status === 'ready' && providerProfiles.profiles.length === 0);
  const projectId = $derived(newConversationProjectId());
  const project = $derived(projectId === null ? undefined : conversationList.findProject(projectId));

  async function start(text: string, mentions: MentionRef[]): Promise<ChatFailure | null> {
    const outcome = await call(() => commands.startConversation({ mentions, profileId: null, projectId, text }));
    if (!outcome.ok) {
      return outcome.failure;
    }
    // The conversation's own pane replaces this one, whose message box the
    // owner wrote in: its message box takes the focus over, and the sidebar
    // follows a move of the conversation before the view shows it.
    markStartedConversation(outcome.data.conversationId);
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
          <Empty.Title>
            {project === undefined ? m.chat_new_title() : m.chat_new_title_in_project({ project: project.name })}
          </Empty.Title>
          <Empty.Description>{m.chat_new_description()}</Empty.Description>
        </Empty.Header>
      </Empty.Root>
    {/if}
  </div>
  <Composer disabled={noProfile} mentionPlace={{ kind: 'new_conversation', projectId }} onSend={start} />
</div>
