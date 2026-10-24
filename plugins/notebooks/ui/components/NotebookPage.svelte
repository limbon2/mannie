<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as DropdownMenu from '@mannie/plugin-sdk/components/dropdown-menu';
  import * as Empty from '@mannie/plugin-sdk/components/empty';
  import * as Item from '@mannie/plugin-sdk/components/item';
  import { Skeleton } from '@mannie/plugin-sdk/components/skeleton';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import BookOpenIcon from '@mannie/plugin-sdk/icons/book-open';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import ClipboardPasteIcon from '@mannie/plugin-sdk/icons/clipboard-paste';
  import EllipsisIcon from '@mannie/plugin-sdk/icons/ellipsis';
  import FilePlusIcon from '@mannie/plugin-sdk/icons/file-plus';
  import FileTextIcon from '@mannie/plugin-sdk/icons/file-text';
  import MessageSquarePlusIcon from '@mannie/plugin-sdk/icons/message-square-plus';
  import TriangleAlertIcon from '@mannie/plugin-sdk/icons/triangle-alert';
  import { openSettings } from '@mannie/plugin-sdk/navigation';

  import { m } from '../generated/messages.js';
  import { notebookActions } from '../state/notebook-actions.svelte.js';
  import { notebookLibrary } from '../state/notebooks.svelte.js';
  import type { FileRefusal } from '../types/notebooks.js';
  import { carriesFiles } from '../utils/notebooks-files.js';
  import { notebooksFailureMessage, sourceRefusalMessage } from '../utils/notebooks-messages.js';
  import { openNotebooks, SETTINGS_SECTION } from '../utils/notebooks-places.js';

  import SourceRow from './SourceRow.svelte';

  // A notebook's page (plugins/notebooks/docs/README.md, notebooks and
  // sources): "New chat", which starts one of the notebook's own chats, the
  // notebook's menu with "Rename…" and "Delete…", and its sources with "Add
  // sources…" and "Paste text…". The sidebar lists the notebooks and their
  // chats. Files dropped on the sources are added; the buttons do the same
  // from the keyboard, and the shell refuses files dropped elsewhere. The
  // shell's heading names the notebook.
  let { notebookId }: { notebookId: string } = $props();

  /** Counts the drag's entries into the sources and their children, so that leaving a child keeps the zone. */
  let dragDepth = $state(0);

  const notebook = $derived(notebookLibrary.find(notebookId));
  const adding = $derived(notebookActions.adding[notebookId] ?? 0);
  const refusals = $derived(notebookActions.refusals[notebookId] ?? []);
  const addFailure = $derived(notebookActions.addFailures[notebookId] ?? null);
  const isPicking = $derived(notebookActions.picking === notebookId);
  const waits = $derived(notebook?.sources.some((source) => source.state === 'waiting') ?? false);

  /** Why a file was not added, as a sentence. */
  function refusalText(refusal: FileRefusal): string {
    const { reason } = refusal;
    return typeof reason === 'string' ? sourceRefusalMessage(reason) : notebooksFailureMessage(reason);
  }

  function dragEnter(event: DragEvent): void {
    if (!carriesFiles(event.dataTransfer)) {
      return;
    }
    event.preventDefault();
    dragDepth += 1;
  }

  function dragOver(event: DragEvent): void {
    if (!carriesFiles(event.dataTransfer)) {
      return;
    }
    event.preventDefault();
    if (event.dataTransfer !== null) {
      event.dataTransfer.dropEffect = 'copy';
    }
  }

  function dragLeave(event: DragEvent): void {
    if (carriesFiles(event.dataTransfer)) {
      dragDepth = Math.max(0, dragDepth - 1);
    }
  }

  function drop(event: DragEvent): void {
    dragDepth = 0;
    if (!carriesFiles(event.dataTransfer)) {
      return;
    }
    event.preventDefault();
    if (notebook !== undefined) {
      void notebookActions.drop(notebook, Array.from(event.dataTransfer?.files ?? []));
    }
  }
</script>

{#if notebookLibrary.status === 'error'}
  <div class="mx-auto w-full max-w-3xl p-6">
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.notebooks_notebook_failed_title()}</Alert.Title>
      <Alert.Description>
        {notebookLibrary.failure === null ? '' : notebooksFailureMessage(notebookLibrary.failure)}
      </Alert.Description>
      <Alert.Action>
        <Button size="sm" variant="outline" onclick={() => notebookLibrary.retry()}>{m.notebooks_try_again()}</Button>
      </Alert.Action>
    </Alert.Root>
  </div>
{:else if notebookLibrary.status === 'loading'}
  <div class="mx-auto flex w-full max-w-3xl flex-col gap-3 p-6" aria-busy="true">
    <span class="sr-only" role="status">{m.notebooks_notebook_loading()}</span>
    <Skeleton class="h-16 w-full" />
    <Skeleton class="h-16 w-full" />
    <Skeleton class="h-16 w-full" />
  </div>
{:else if notebook === undefined}
  <Empty.Root class="flex-1" data-notebooks-missing="">
    <Empty.Header>
      <Empty.Media variant="icon">
        <BookOpenIcon />
      </Empty.Media>
      <Empty.Title>{m.notebooks_notebook_missing_title()}</Empty.Title>
      <Empty.Description>{m.notebooks_notebook_missing_description()}</Empty.Description>
    </Empty.Header>
    <Empty.Content>
      <Button variant="outline" onclick={() => void openNotebooks()}>{m.notebooks_open_latest()}</Button>
    </Empty.Content>
  </Empty.Root>
{:else}
  <div class="mx-auto flex w-full max-w-3xl flex-col gap-6 p-6" data-notebooks-page={notebook.id}>
    <div class="flex flex-wrap items-center gap-2">
      <div class="ms-auto flex flex-wrap items-center gap-2">
        <Button data-notebooks-new-chat="" onclick={() => void notebookActions.newChat(notebook)}>
          <MessageSquarePlusIcon data-icon="inline-start" />
          {m.notebooks_new_chat()}
        </Button>
        <DropdownMenu.Root>
          <DropdownMenu.Trigger>
            {#snippet child({ props })}
              <Button
                {...props}
                aria-label={m.notebooks_notebook_menu({ name: notebook.name })}
                data-notebooks-notebook-menu=""
                size="icon"
                variant="outline"
              >
                <EllipsisIcon />
              </Button>
            {/snippet}
          </DropdownMenu.Trigger>
          <DropdownMenu.Content align="end">
            <DropdownMenu.Group>
              <DropdownMenu.Item data-notebooks-rename="" onSelect={() => notebookActions.startRename(notebook)}>
                {m.notebooks_rename()}
              </DropdownMenu.Item>
            </DropdownMenu.Group>
            <DropdownMenu.Separator />
            <DropdownMenu.Group>
              <DropdownMenu.Item
                data-notebooks-delete=""
                variant="destructive"
                onSelect={() => notebookActions.startDelete(notebook)}
              >
                {m.notebooks_delete()}
              </DropdownMenu.Item>
            </DropdownMenu.Group>
          </DropdownMenu.Content>
        </DropdownMenu.Root>
      </div>
    </div>
    {#if waits}
      <Alert.Root data-notebooks-waiting="">
        <TriangleAlertIcon />
        <Alert.Title>{m.notebooks_waiting_title()}</Alert.Title>
        <Alert.Description>{m.notebooks_waiting_description()}</Alert.Description>
        <Alert.Action>
          <Button size="sm" variant="outline" onclick={() => void openSettings(SETTINGS_SECTION)}>
            {m.notebooks_open_settings()}
          </Button>
        </Alert.Action>
      </Alert.Root>
    {/if}
    <section
      class={[
        'flex flex-col gap-3 rounded-lg',
        dragDepth > 0 && 'outline-2 outline-offset-8 outline-primary outline-dashed',
      ]}
      aria-labelledby="notebooks-sources-heading"
      data-notebooks-sources=""
      ondragenter={dragEnter}
      ondragleave={dragLeave}
      ondragover={dragOver}
      ondrop={drop}
    >
      <div class="flex flex-wrap items-center gap-2">
        <h2 id="notebooks-sources-heading" class="text-base font-semibold">{m.notebooks_sources_heading()}</h2>
        <div class="ms-auto flex flex-wrap items-center gap-2">
          <Button
            data-notebooks-add-sources=""
            disabled={isPicking}
            size="sm"
            variant="outline"
            onclick={() => void notebookActions.pick(notebook)}
          >
            <FilePlusIcon data-icon="inline-start" />
            {m.notebooks_add_sources()}
          </Button>
          <Button
            data-notebooks-paste-text=""
            size="sm"
            variant="outline"
            onclick={() => notebookActions.startPaste(notebook)}
          >
            <ClipboardPasteIcon data-icon="inline-start" />
            {m.notebooks_paste_text()}
          </Button>
        </div>
      </div>
      <div class="text-sm text-muted-foreground" role="status">
        {#if dragDepth > 0}
          <p class="text-primary">{m.notebooks_drop_hint()}</p>
        {:else if adding > 0}
          <p class="flex items-center gap-2">
            <Spinner aria-hidden="true" />
            <span>{m.notebooks_adding_files({ count: adding })}</span>
          </p>
        {/if}
      </div>
      {#if addFailure !== null}
        <Alert.Root variant="destructive">
          <CircleAlertIcon />
          <Alert.Title>{m.notebooks_add_failed_title()}</Alert.Title>
          <Alert.Description>{notebooksFailureMessage(addFailure)}</Alert.Description>
        </Alert.Root>
      {/if}
      {#if refusals.length > 0}
        <Alert.Root data-notebooks-refused="" variant="destructive">
          <CircleAlertIcon />
          <Alert.Title>{m.notebooks_refused_title({ count: refusals.length })}</Alert.Title>
          <Alert.Description>
            <ul class="flex flex-col gap-1">
              {#each refusals as refusal, index (index)}
                <li>
                  <span class="font-medium wrap-anywhere" data-content dir="auto">{refusal.name}</span>
                  <span>{refusalText(refusal)}</span>
                </li>
              {/each}
            </ul>
          </Alert.Description>
          <Alert.Action>
            <Button size="sm" variant="outline" onclick={() => notebookActions.dismissRefusals(notebook.id)}>
              {m.notebooks_dismiss()}
            </Button>
          </Alert.Action>
        </Alert.Root>
      {/if}
      {#if notebook.sources.length === 0}
        <Empty.Root class="border border-dashed" data-notebooks-no-sources="">
          <Empty.Header>
            <Empty.Media variant="icon">
              <FileTextIcon />
            </Empty.Media>
            <Empty.Title>{m.notebooks_sources_empty_title()}</Empty.Title>
            <Empty.Description>{m.notebooks_sources_empty_description()}</Empty.Description>
          </Empty.Header>
        </Empty.Root>
      {:else}
        <Item.Group aria-label={m.notebooks_sources_label({ name: notebook.name })}>
          {#each notebook.sources as source (source.id)}
            <SourceRow {source} />
          {/each}
        </Item.Group>
      {/if}
    </section>
  </div>
{/if}
