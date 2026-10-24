<script lang="ts">
  import MessagesSquareIcon from '@lucide/svelte/icons/messages-square';

  import * as Empty from '#lib/components/ui/empty/index.js';
  import { type Attachment, commands, type ImageRef, type MentionRef } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { conversationDraft } from '#lib/state/conversation-draft.svelte.js';
  import { conversationList } from '#lib/state/conversation-list.svelte.js';
  import { providerProfiles } from '#lib/state/provider-profiles.svelte.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import { markStartedConversation } from '#lib/state/started-conversation.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import type { ConversationDraft } from '#lib/types/shell-slots.js';
  import { call } from '#lib/utils/calls.js';
  import { newConversationProjectId, openConversation } from '#lib/utils/chat-urls.js';
  import { controlsIn } from '#lib/utils/conversation-controls.js';

  import AttachmentChips from './AttachmentChips.svelte';
  import Composer from './Composer.svelte';
  import NoProfileNotice from './NoProfileNotice.svelte';

  // A new conversation: the message box, with a hint about what the owner can
  // do. The first message creates the conversation, which then opens. A new
  // conversation of a project, which the address names, starts in it and
  // works in its folder (docs/features/chat/projects.md); the heading names
  // the project, and nothing changes near the message box. The controls of
  // the message box that work on a draft, such as a plugin's attachments,
  // show here too, with the attachments that the conversation starts with
  // (ADR-0046, decision 6). A conversation that a plugin starts for one of
  // its items, such as a notebook's chat, belongs to it: the heading names
  // it, and its chip stays (ADR-0051).
  const slots = getShellSlots();

  const noProfile = $derived(providerProfiles.status === 'ready' && providerProfiles.profiles.length === 0);
  const projectId = $derived(conversationDraft.home === null ? newConversationProjectId() : null);
  const project = $derived(projectId === null ? undefined : conversationList.findProject(projectId));
  /** The label of the attachment that the conversation belongs to, which the draft holds first. */
  const homeLabel = $derived(conversationDraft.home === null ? undefined : conversationDraft.attachments[0]?.label);
  const title = $derived.by(() => {
    if (homeLabel !== undefined) {
      return m.chat_new_title_in_home({ label: homeLabel });
    }
    return project === undefined ? m.chat_new_title() : m.chat_new_title_in_project({ project: project.name });
  });

  /** The controls of the message box that work on a draft, in their order. */
  const draftControls = $derived(
    controlsIn(slots.conversationControls.entries, 'message-box').flatMap((control) =>
      control.draft === undefined ? [] : [{ Draft: control.draft, id: control.id }],
    ),
  );

  const draft: ConversationDraft = {
    attach: (attachment) => {
      conversationDraft.attach(attachment);
    },
    get attachments() {
      return conversationDraft.attachments;
    },
    detach: (attachment) => {
      conversationDraft.detach(attachment);
    },
    get projectId() {
      return projectId;
    },
  };

  async function start(text: string, mentions: MentionRef[], images: ImageRef[]): Promise<ChatFailure | null> {
    const attachments = conversationDraft.refs();
    const home = conversationDraft.home;
    const outcome = await call(() =>
      commands.startConversation({ attachments, home, images, mentions, profileId: null, projectId, text }),
    );
    if (!outcome.ok) {
      return outcome.failure;
    }
    conversationDraft.reset();
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
          <Empty.Title>{title}</Empty.Title>
          <Empty.Description>{m.chat_new_description()}</Empty.Description>
        </Empty.Header>
      </Empty.Root>
    {/if}
  </div>
  <Composer disabled={noProfile} mentionPlace={{ kind: 'new_conversation', projectId }} onSend={start}>
    {#snippet controls()}
      {#each draftControls as control (control.id)}
        <control.Draft {draft} />
      {/each}
    {/snippet}
    {#snippet attached()}
      {#if conversationDraft.attachments.length > 0}
        <AttachmentChips
          attachments={conversationDraft.attachments}
          home={conversationDraft.home}
          ondetach={(attachment: Attachment) => {
            conversationDraft.detach(attachment);
          }}
        />
      {/if}
    {/snippet}
  </Composer>
</div>
