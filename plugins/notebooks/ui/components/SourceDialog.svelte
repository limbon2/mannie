<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import XIcon from '@mannie/plugin-sdk/icons/x';

  import { m } from '../generated/messages.js';
  import { notebookLibrary } from '../state/notebooks.svelte.js';
  import { sourceViewer } from '../state/source-viewer.svelte.js';

  import SourceViewer from './SourceViewer.svelte';

  // The viewer of a source (plugins/notebooks/docs/README.md, sources and
  // answers): a dialog that fills the window, over the view or the chat that
  // opened it, which Close and Escape close. Each request shows a new viewer,
  // which reads the source again and scrolls to the passage that it names.
  const request = $derived(sourceViewer.request);
  const source = $derived(request === null ? undefined : notebookLibrary.findSource(request.sourceId));

  /** Gives the focus back to the element that opened the viewer, when it named one that is still there. */
  function returnFocus(event: Event): void {
    const opener = request?.opener ?? null;
    if (opener?.isConnected === true) {
      event.preventDefault();
      opener.focus();
    }
  }
</script>

<Dialog.Root bind:open={sourceViewer.open}>
  <Dialog.Content
    class="flex size-[calc(100%-2rem)] max-w-none flex-col sm:max-w-none"
    data-notebooks-source-dialog=""
    showCloseButton={false}
    onCloseAutoFocus={returnFocus}
  >
    {#if request !== null}
      <div class="flex items-start gap-2">
        <Dialog.Header class="min-w-0 flex-1">
          <Dialog.Title class="wrap-anywhere" data-content dir="auto">
            {source?.name ?? m.notebooks_viewer_title()}
          </Dialog.Title>
          <Dialog.Description class="sr-only">{m.notebooks_viewer_description()}</Dialog.Description>
        </Dialog.Header>
        <Button
          aria-label={m.notebooks_close()}
          data-notebooks-viewer-close=""
          size="icon-sm"
          variant="ghost"
          onclick={() => sourceViewer.close()}
        >
          <XIcon />
        </Button>
      </div>
      {#key request}
        <SourceViewer {request} onclose={() => sourceViewer.close()} />
      {/key}
    {/if}
  </Dialog.Content>
</Dialog.Root>
