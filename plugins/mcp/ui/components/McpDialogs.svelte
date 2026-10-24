<script lang="ts">
  import { onDestroy } from 'svelte';

  import { mcpActions } from '../state/mcp-actions.svelte.js';

  import PasteConfigDialog from './PasteConfigDialog.svelte';
  import RemoveServerDialog from './RemoveServerDialog.svelte';
  import ServerDialog from './ServerDialog.svelte';
  import ServerLogDialog from './ServerLogDialog.svelte';
  import ServerToolsDialog from './ServerToolsDialog.svelte';

  // The MCP plugin's dialogs, which the settings and the command palette
  // open. The shell mounts them once, as an overlay (ADR-0020), while the
  // plugin is on. They close as the plugin turns off, so that turning it on
  // again does not open them again.
  onDestroy(() => {
    mcpActions.closeAll();
  });
</script>

<ServerDialog />
<ServerToolsDialog />
<ServerLogDialog />
<RemoveServerDialog />
<PasteConfigDialog />
