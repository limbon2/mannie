<script lang="ts">
  import ConversationPermissionsDialog from './ConversationPermissionsDialog.svelte';
  import FullAccessDialog from './FullAccessDialog.svelte';
  import SaveRuleDialog from './SaveRuleDialog.svelte';
  import ToolOutputDialog from './ToolOutputDialog.svelte';

  // The system tools feature's dialogs, which the tool call rows, the
  // command palette and the full access switch open. The shell mounts them
  // once, as an overlay (ADR-0020).
</script>

<SaveRuleDialog />
<ToolOutputDialog />
<ConversationPermissionsDialog />
<FullAccessDialog />
