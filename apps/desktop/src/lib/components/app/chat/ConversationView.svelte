<script lang="ts">
  import { onMount } from 'svelte';

  import { page } from '$app/state';

  import { conversationList } from '#lib/state/conversation-list.svelte.js';
  import { providerProfiles } from '#lib/state/provider-profiles.svelte.js';
  import { CONVERSATION_PARAMETER } from '#lib/utils/chat-urls.js';

  import ConversationPane from './ConversationPane.svelte';
  import NewConversationPane from './NewConversationPane.svelte';

  // The chat view (docs/features/chat/conversation-view.md): the conversation
  // that the address names, or a new one.
  const conversationId = $derived(page.url.searchParams.get(CONVERSATION_PARAMETER));

  onMount(() => {
    conversationList.start();
    void providerProfiles.ensureLoaded();
  });
</script>

{#key conversationId}
  {#if conversationId === null}
    <NewConversationPane />
  {:else}
    <ConversationPane id={conversationId} />
  {/if}
{/key}
