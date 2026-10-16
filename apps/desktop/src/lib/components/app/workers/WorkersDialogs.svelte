<script lang="ts">
  import PairWorkerDialog from './PairWorkerDialog.svelte';
  import RemoveWorkerDialog from './RemoveWorkerDialog.svelte';
  import RenameWorkerDialog from './RenameWorkerDialog.svelte';

  // The workers feature's dialogs, which the sidebar, the command palette
  // and the Workers settings open. The shell mounts them once, as an
  // overlay (ADR-0020).
</script>

<PairWorkerDialog />
<RenameWorkerDialog />
<RemoveWorkerDialog />
