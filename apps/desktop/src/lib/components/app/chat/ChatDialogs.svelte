<script lang="ts">
  import CheckpointDialog from './CheckpointDialog.svelte';
  import DeleteConversationDialog from './DeleteConversationDialog.svelte';
  import RenameConversationDialog from './RenameConversationDialog.svelte';

  // The chat feature's dialogs, which the command palette, the conversation
  // list and the conversation view open. The shell mounts them once, as an
  // overlay (ADR-0020).
</script>

<RenameConversationDialog />
<DeleteConversationDialog />
<CheckpointDialog />
