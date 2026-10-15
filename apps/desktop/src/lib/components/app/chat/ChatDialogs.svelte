<script lang="ts">
  import AddProjectPathDialog from './AddProjectPathDialog.svelte';
  import CheckpointDialog from './CheckpointDialog.svelte';
  import DeleteConversationDialog from './DeleteConversationDialog.svelte';
  import RemoveProjectDialog from './RemoveProjectDialog.svelte';
  import RenameConversationDialog from './RenameConversationDialog.svelte';
  import RenameProjectDialog from './RenameProjectDialog.svelte';

  // The chat feature's dialogs, which the command palette, the sidebar and
  // the conversation view open. The shell mounts them once, as an overlay
  // (ADR-0020).
</script>

<RenameConversationDialog />
<DeleteConversationDialog />
<CheckpointDialog />
<RenameProjectDialog />
<RemoveProjectDialog />
<AddProjectPathDialog />
