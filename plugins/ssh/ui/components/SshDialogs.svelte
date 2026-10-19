<script lang="ts">
  import { onDestroy } from 'svelte';

  import { sshActions } from '../state/ssh-actions.svelte.js';

  import HostDialog from './HostDialog.svelte';
  import HostKeyDialog from './HostKeyDialog.svelte';
  import RemoveHostDialog from './RemoveHostDialog.svelte';

  // The SSH plugin's dialogs, which the settings and the command palette
  // open. The shell mounts them once, as an overlay (ADR-0020), while the
  // plugin is on. They close as the plugin turns off, so that turning it on
  // again does not open them again.
  onDestroy(() => {
    sshActions.closeAll();
  });
</script>

<HostDialog />
<HostKeyDialog />
<RemoveHostDialog />
