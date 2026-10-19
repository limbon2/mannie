<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Empty from '@mannie/plugin-sdk/components/empty';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import * as Item from '@mannie/plugin-sdk/components/item';
  import { Skeleton } from '@mannie/plugin-sdk/components/skeleton';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import NotepadTextIcon from '@mannie/plugin-sdk/icons/notepad-text';

  import { m } from '../generated/messages.js';
  import { MAX_NOTE_CHARACTERS } from '../generated/plugin.js';
  import { noteList } from '../state/notes.svelte.js';
  import { notesFailureMessage } from '../utils/notes-messages.js';

  // The plugin's settings section: a field that adds a note through the
  // plugin's command, and the notes that the plugin kept since it turned
  // on, which its channel brings. "Add note"
  // waits for text, the field takes no more than the plugin keeps, and a
  // note that was not added keeps its text in the field, with the reason.
  let text = $state('');

  const isBlank = $derived(text.trim() === '');

  async function add(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (noteList.adding || isBlank) {
      return;
    }
    const kept = await noteList.add(text);
    if (kept) {
      text = '';
    }
  }
</script>

<div class="flex flex-col gap-6" data-example-settings="">
  <form class="flex flex-col gap-3" onsubmit={(event) => void add(event)}>
    <Field.Field>
      <Field.Label for="example-note">{m.example_note_label()}</Field.Label>
      <Input
        id="example-note"
        autocomplete="off"
        disabled={noteList.adding}
        maxlength={MAX_NOTE_CHARACTERS}
        bind:value={text}
      />
    </Field.Field>
    <div>
      <Button data-example-add-note="" disabled={noteList.adding || isBlank} type="submit">
        {m.example_add_note()}
      </Button>
    </div>
    <div role="status">
      {#if noteList.adding}
        <p class="flex items-center gap-2 text-sm text-muted-foreground">
          <Spinner aria-hidden="true" />
          <span>{m.example_adding()}</span>
        </p>
      {/if}
    </div>
    {#if noteList.addFailure !== null}
      <Alert.Root data-example-add-failed="" variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.example_add_failed_title()}</Alert.Title>
        <Alert.Description>{notesFailureMessage(noteList.addFailure)}</Alert.Description>
      </Alert.Root>
    {/if}
  </form>
  {#if noteList.status === 'error'}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.example_notes_failed_title()}</Alert.Title>
      <Alert.Description>{noteList.failure === null ? '' : notesFailureMessage(noteList.failure)}</Alert.Description>
      <Alert.Action>
        <Button size="sm" variant="outline" onclick={() => noteList.retry()}>{m.example_try_again()}</Button>
      </Alert.Action>
    </Alert.Root>
  {:else if noteList.status === 'loading'}
    <div class="flex flex-col gap-3" aria-busy="true">
      <span class="sr-only" role="status">{m.example_loading()}</span>
      <Skeleton class="h-12 w-full" />
    </div>
  {:else if noteList.notes.length === 0}
    <Empty.Root class="border">
      <Empty.Header>
        <Empty.Media variant="icon">
          <NotepadTextIcon />
        </Empty.Media>
        <Empty.Title>{m.example_notes_empty_title()}</Empty.Title>
        <Empty.Description>{m.example_notes_empty_description()}</Empty.Description>
      </Empty.Header>
    </Empty.Root>
  {:else}
    <Item.Group aria-label={m.example_notes_label()}>
      {#each noteList.notes as note (note.id)}
        <Item.Root data-example-note={note.id} role="listitem" variant="outline">
          <Item.Content class="min-w-0">
            <Item.Description class="wrap-anywhere whitespace-pre-wrap" data-content dir="auto">
              {note.text}
            </Item.Description>
          </Item.Content>
        </Item.Root>
      {/each}
    </Item.Group>
  {/if}
</div>
