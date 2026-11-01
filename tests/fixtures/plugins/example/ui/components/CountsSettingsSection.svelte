<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { Skeleton } from '@mannie/plugin-sdk/components/skeleton';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';

  import { m } from '../generated/messages.js';
  import { noteCount } from '../state/counts.svelte.js';
  import { notesFailureMessage } from '../utils/notes-messages.js';

  // The settings section of the plugin's optional feature `counts`, which
  // shows only while the plugin and the feature are on: how many notes the
  // plugin kept since it started, and "Count again".
</script>

<div class="flex flex-col gap-4" data-example-counts="">
  {#if noteCount.status === 'error'}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.example_count_failed_title()}</Alert.Title>
      <Alert.Description>{noteCount.failure === null ? '' : notesFailureMessage(noteCount.failure)}</Alert.Description>
      <Alert.Action>
        <Button size="sm" variant="outline" onclick={() => void noteCount.refresh()}>{m.example_try_again()}</Button>
      </Alert.Action>
    </Alert.Root>
  {:else if noteCount.count === null}
    <div class="flex flex-col gap-3" aria-busy="true">
      <span class="sr-only" role="status">{m.example_counting()}</span>
      <Skeleton class="h-6 w-48" />
    </div>
  {:else}
    <p class="text-sm" aria-busy={noteCount.status === 'loading'} data-example-count={noteCount.count} role="status">
      {m.example_count_kept({ count: noteCount.count })}
    </p>
    <div>
      <Button data-example-count-again="" size="sm" variant="outline" onclick={() => void noteCount.refresh()}>
        {m.example_count_again()}
      </Button>
    </div>
  {/if}
</div>
