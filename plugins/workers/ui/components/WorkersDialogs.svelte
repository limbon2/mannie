<script lang="ts">
  import { onDestroy } from 'svelte';

  import { workersActions } from '../state/workers-actions.svelte.js';

  import PairWorkerDialog from './PairWorkerDialog.svelte';
  import RemoveWorkerDialog from './RemoveWorkerDialog.svelte';
  import RenameWorkerDialog from './RenameWorkerDialog.svelte';

  // The workers feature's dialogs, which the sidebar, the command palette
  // and the Workers settings open. The shell mounts them once, as an
  // overlay (ADR-0020), while Workers is on. They close as Workers turns
  // off, so that turning it on again does not open them again.
  onDestroy(() => {
    workersActions.closeAll();
  });
</script>

<PairWorkerDialog />
<RenameWorkerDialog />
<RemoveWorkerDialog />
