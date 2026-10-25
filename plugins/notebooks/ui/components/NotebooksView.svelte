<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Empty from '@mannie/plugin-sdk/components/empty';
  import { Skeleton } from '@mannie/plugin-sdk/components/skeleton';
  import BookOpenIcon from '@mannie/plugin-sdk/icons/book-open';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';

  import { m } from '../generated/messages.js';
  import { notebookActions } from '../state/notebook-actions.svelte.js';
  import { notebookLibrary } from '../state/notebooks.svelte.js';
  import { notebooksFailureMessage } from '../utils/notebooks-messages.js';
  import { openNotebook, shownNotebookId } from '../utils/notebooks-places.js';

  import NotebookPage from './NotebookPage.svelte';

  // The plugin's view (plugins/notebooks/docs/README.md, interface): one
  // notebook's page, which the parameter `notebook` names. The sidebar lists
  // the notebooks, so the view without a notebook leads to the most recently
  // changed one, replacing its address so that Back skips it, or, without
  // notebooks, offers "New notebook". The shell's heading names the view, or
  // the notebook that it shows.
  const notebookId = $derived(shownNotebookId());
  const isBare = $derived(notebookId === null);
  const isReady = $derived(notebookLibrary.status === 'ready');
  const isLoading = $derived(notebookLibrary.status === 'loading');
  /** The notebook that the view without a notebook leads to, once the notebooks are known. */
  const target = $derived(isBare && isReady ? notebookLibrary.latest() : undefined);
  const leads = $derived(target !== undefined);
  /** Whether the view waits for the notebooks, or for the page that it leads to. */
  const waits = $derived(isLoading || leads);

  // Navigation is the effect's whole work: the view leads to a page.
  $effect(() => {
    if (target !== undefined) {
      void openNotebook(target.id, { replace: true });
    }
  });
</script>

{#if notebookId !== null}
  {#key notebookId}
    <NotebookPage {notebookId} />
  {/key}
{:else if notebookLibrary.status === 'error'}
  <div class="mx-auto w-full max-w-3xl p-6">
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.notebooks_list_failed_title()}</Alert.Title>
      <Alert.Description>
        {notebookLibrary.failure === null ? '' : notebooksFailureMessage(notebookLibrary.failure)}
      </Alert.Description>
      <Alert.Action>
        <Button size="sm" variant="outline" onclick={() => notebookLibrary.retry()}>{m.notebooks_try_again()}</Button>
      </Alert.Action>
    </Alert.Root>
  </div>
{:else if waits}
  <div class="mx-auto flex w-full max-w-3xl flex-col gap-3 p-6" aria-busy="true">
    <span class="sr-only" role="status">{m.notebooks_loading()}</span>
    <Skeleton class="h-16 w-full" />
    <Skeleton class="h-16 w-full" />
  </div>
{:else}
  <Empty.Root class="flex-1" data-notebooks-none="">
    <Empty.Header>
      <Empty.Media variant="icon">
        <BookOpenIcon />
      </Empty.Media>
      <Empty.Title>{m.notebooks_empty_title()}</Empty.Title>
      <Empty.Description>{m.notebooks_empty_description()}</Empty.Description>
    </Empty.Header>
    <Empty.Content>
      <Button data-notebooks-new="" onclick={() => notebookActions.startCreate()}>
        <PlusIcon data-icon="inline-start" />
        {m.notebooks_new_notebook()}
      </Button>
    </Empty.Content>
  </Empty.Root>
{/if}
