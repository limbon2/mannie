<script lang="ts">
  import { onDestroy } from 'svelte';

  import { projectActions } from '../state/project-actions.svelte.js';

  import ProjectNameDialog from './ProjectNameDialog.svelte';

  // The plugin's dialogs (ADR-0045, an overlay): mounted once for the life of
  // the window, so that the sidebar opens them wherever the owner is. They
  // close as the plugin turns off, so that turning it on again does not open
  // them again.
  onDestroy(() => {
    projectActions.closeAll();
  });
</script>

<ProjectNameDialog />
