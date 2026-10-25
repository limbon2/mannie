<script lang="ts">
  import { onDestroy } from 'svelte';

  import { notebookActions } from '../state/notebook-actions.svelte.js';
  import { sourceViewer } from '../state/source-viewer.svelte.js';

  import DeleteNotebookDialog from './DeleteNotebookDialog.svelte';
  import NotebookNameDialog from './NotebookNameDialog.svelte';
  import PasteTextDialog from './PasteTextDialog.svelte';
  import RemoveSourceDialog from './RemoveSourceDialog.svelte';
  import SourceDialog from './SourceDialog.svelte';

  // The Notebooks plugin's dialogs, which its view, its sidebar group, the
  // citations of the transcript and the command palette open. The shell
  // mounts them once, as an overlay (ADR-0020), while the plugin is on. They
  // close as the plugin turns off, so that turning it on again does not open
  // them again.
  onDestroy(() => {
    notebookActions.closeAll();
    sourceViewer.close();
  });
</script>

<NotebookNameDialog />
<DeleteNotebookDialog />
<PasteTextDialog />
<RemoveSourceDialog />
<SourceDialog />
