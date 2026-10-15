<script lang="ts">
  import ConversationPermissionsDialog from './ConversationPermissionsDialog.svelte';
  import SaveRuleDialog from './SaveRuleDialog.svelte';
  import ToolOutputDialog from './ToolOutputDialog.svelte';

  // The system tools feature's dialogs, which the tool call rows and the
  // command palette open. The shell mounts them once, as an overlay
  // (ADR-0020).
</script>

<SaveRuleDialog />
<ToolOutputDialog />
<ConversationPermissionsDialog />
