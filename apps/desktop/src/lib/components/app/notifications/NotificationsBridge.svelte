<script lang="ts">
  import { onMount } from 'svelte';

  import { notifications } from '#lib/state/notifications.svelte.js';
  import { openConversationId } from '#lib/utils/chat-urls.js';

  // Root overlay for desktop notifications and unread finished conversations
  // (docs/features/notifications.md; ADR-0074). Sends localized toast titles
  // from the message catalog, subscribes to unread updates and notification
  // clicks, and reports whichever conversation the main view currently shows.
  onMount(() => {
    notifications.start();
    notifications.syncLabels();
  });

  $effect(() => {
    notifications.reportViewed(openConversationId());
  });
</script>
