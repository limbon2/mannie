<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Empty from '@mannie/plugin-sdk/components/empty';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import * as Select from '@mannie/plugin-sdk/components/select';
  import { Skeleton } from '@mannie/plugin-sdk/components/skeleton';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import BookOpenIcon from '@mannie/plugin-sdk/icons/book-open';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import { openProvidersPage } from '@mannie/plugin-sdk/navigation';
  import { onMount } from 'svelte';

  import { m } from '../generated/messages.js';
  import { FEWEST_PASSAGES, MOST_PASSAGES } from '../generated/plugin.js';
  import { notebookSettings } from '../state/notebook-settings.svelte.js';
  import { notebooksFailureMessage } from '../utils/notebooks-messages.js';
  import {
    formOf,
    isValidPassages,
    preferencesOf,
    type ProfileChoice,
    profileChoices,
    type SettingsForm,
  } from '../utils/notebooks-settings.js';

  // The Notebooks section of the settings (plugins/notebooks/docs/README.md,
  // the models): the embedding model and the reranker, picked from the
  // owner's search profiles, and the most passages that a message gets. The
  // reranker orders passages and drops none (ADR-0049). Without an embedding
  // profile, the section leads to the Providers settings, where the owner
  // adds one. A new embedding model indexes every source again, which the
  // plugin does once the owner saves.
  let form = $state<SettingsForm | null>(null);

  const profiles = $derived(notebookSettings.profiles);
  const embeddings = $derived(profileChoices(profiles?.embeddings ?? []));
  const reranks = $derived(profileChoices(profiles?.rerank ?? []));
  const hasEmbeddings = $derived((profiles?.embeddings.length ?? 0) > 0);
  const passagesInvalid = $derived(form !== null && !isValidPassages(form.maxPassages));
  const range = { fewest: formatCount(FEWEST_PASSAGES), most: formatCount(MOST_PASSAGES) };

  /** The label of the chosen value, which the trigger shows. */
  function labelOf(choices: readonly ProfileChoice[], value: string): string {
    return choices.find((choice) => choice.value === value)?.label ?? '';
  }

  async function load(): Promise<void> {
    form = null;
    await notebookSettings.load();
    const { preferences, profiles: read } = notebookSettings;
    const wasRead = preferences !== null && read !== null;
    if (wasRead) {
      form = formOf(preferences, read.embeddings, read.rerank);
    }
  }

  async function save(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const preferences = form === null ? null : preferencesOf(form);
    if (preferences === null) {
      return;
    }
    if (!notebookSettings.saving) {
      await notebookSettings.save(preferences);
    }
  }

  onMount(() => {
    void load();
  });
</script>

<div class="flex flex-col gap-6" data-notebooks-settings="">
  {#if notebookSettings.status === 'error'}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.notebooks_settings_failed_title()}</Alert.Title>
      <Alert.Description>
        {notebookSettings.readFailure === null ? '' : notebooksFailureMessage(notebookSettings.readFailure)}
      </Alert.Description>
      <Alert.Action>
        <Button size="sm" variant="outline" onclick={() => void load()}>{m.notebooks_try_again()}</Button>
      </Alert.Action>
    </Alert.Root>
  {:else if form === null}
    <div class="flex flex-col gap-3" aria-busy="true">
      <span class="sr-only" role="status">{m.notebooks_settings_loading()}</span>
      <Skeleton class="h-9 w-full" />
      <Skeleton class="h-9 w-full" />
    </div>
  {:else}
    {#if !hasEmbeddings}
      <Empty.Root class="border" data-notebooks-no-profiles="">
        <Empty.Header>
          <Empty.Media variant="icon">
            <BookOpenIcon />
          </Empty.Media>
          <Empty.Title>{m.notebooks_settings_no_profiles_title()}</Empty.Title>
          <Empty.Description>{m.notebooks_settings_no_profiles_description()}</Empty.Description>
        </Empty.Header>
        <Empty.Content>
          <Button data-notebooks-open-providers="" onclick={() => void openProvidersPage()}>
            {m.notebooks_settings_open_providers()}
          </Button>
        </Empty.Content>
      </Empty.Root>
    {/if}
    <form class="flex flex-col gap-6" onsubmit={(event) => void save(event)}>
      <Field.Group>
        <Field.Field>
          <Field.Label for="notebooks-embedding">{m.notebooks_settings_embedding_label()}</Field.Label>
          <Select.Root items={embeddings} type="single" bind:value={form.embedding}>
            <Select.Trigger
              id="notebooks-embedding"
              class="w-full"
              aria-describedby="notebooks-embedding-description"
              data-notebooks-embedding=""
            >
              <span class="truncate" data-content>{labelOf(embeddings, form.embedding)}</span>
            </Select.Trigger>
            <Select.Content>
              {#each embeddings as choice (choice.value)}
                <Select.Item label={choice.label} value={choice.value}>
                  <span class="truncate" data-content>{choice.label}</span>
                </Select.Item>
              {/each}
            </Select.Content>
          </Select.Root>
          <Field.Description id="notebooks-embedding-description">
            {m.notebooks_settings_embedding_description()}
          </Field.Description>
        </Field.Field>
        <Field.Field>
          <Field.Label for="notebooks-rerank">{m.notebooks_settings_rerank_label()}</Field.Label>
          <Select.Root items={reranks} type="single" bind:value={form.rerank}>
            <Select.Trigger
              id="notebooks-rerank"
              class="w-full"
              aria-describedby="notebooks-rerank-description"
              data-notebooks-rerank=""
            >
              <span class="truncate" data-content>{labelOf(reranks, form.rerank)}</span>
            </Select.Trigger>
            <Select.Content>
              {#each reranks as choice (choice.value)}
                <Select.Item label={choice.label} value={choice.value}>
                  <span class="truncate" data-content>{choice.label}</span>
                </Select.Item>
              {/each}
            </Select.Content>
          </Select.Root>
          <Field.Description id="notebooks-rerank-description">
            {m.notebooks_settings_rerank_description()}
          </Field.Description>
        </Field.Field>
        <Field.Field data-invalid={passagesInvalid ? true : undefined}>
          <Field.Label for="notebooks-passages">{m.notebooks_settings_passages_label()}</Field.Label>
          <Input
            id="notebooks-passages"
            class="w-full max-w-xs"
            aria-describedby="notebooks-passages-description"
            aria-invalid={passagesInvalid}
            max={MOST_PASSAGES}
            min={FEWEST_PASSAGES}
            step="1"
            type="number"
            bind:value={form.maxPassages}
          />
          <Field.Description id="notebooks-passages-description">
            {m.notebooks_settings_passages_description(range)}
          </Field.Description>
          {#if passagesInvalid}
            <Field.Error>{m.notebooks_settings_invalid_passages(range)}</Field.Error>
          {/if}
        </Field.Field>
      </Field.Group>
      <div class="flex flex-wrap items-center gap-3">
        <Button data-notebooks-save-settings="" disabled={notebookSettings.saving || passagesInvalid} type="submit">
          {m.notebooks_settings_save()}
        </Button>
        <div class="text-sm text-muted-foreground" role="status">
          {#if notebookSettings.saving}
            <p class="flex items-center gap-2">
              <Spinner aria-hidden="true" />
              <span>{m.notebooks_settings_saving()}</span>
            </p>
          {:else if notebookSettings.saved}
            <p data-notebooks-settings-saved="">{m.notebooks_settings_saved()}</p>
          {/if}
        </div>
      </div>
      {#if notebookSettings.saveFailure !== null}
        <Alert.Root variant="destructive">
          <CircleAlertIcon />
          <Alert.Title>{m.notebooks_settings_save_failed_title()}</Alert.Title>
          <Alert.Description>{notebooksFailureMessage(notebookSettings.saveFailure)}</Alert.Description>
        </Alert.Root>
      {/if}
    </form>
  {/if}
</div>
