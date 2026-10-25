<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
  import PlusIcon from '@lucide/svelte/icons/plus';
  import { onMount } from 'svelte';

  import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Badge } from '#lib/components/ui/badge/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
  import * as Empty from '#lib/components/ui/empty/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import type { SearchProfile } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { SearchProfileForm } from '#lib/state/search-profile-form.svelte.js';
  import { searchProfiles } from '#lib/state/search-profiles.svelte.js';
  import { SearchTestRun } from '#lib/state/search-test-run.svelte.js';
  import type { ProfileFailure } from '#lib/types/chat.js';
  import { profileFailureMessage } from '#lib/utils/chat-errors.js';
  import { searchKindLabel, serverLabel } from '#lib/utils/provider-labels.js';

  import SearchProfileDialog from './SearchProfileDialog.svelte';
  import SearchTestDialog from './SearchTestDialog.svelte';

  // The search profiles below the chat profiles (ADR-0047): each embedding or
  // rerank model with its kind and server, and Test, Edit and Delete.
  let editorOpen = $state(false);
  let editor = $state<SearchProfileForm | null>(null);
  let testOpen = $state(false);
  let testRun = $state<SearchTestRun | null>(null);
  let deleting = $state<SearchProfile | null>(null);
  let actionFailure = $state<ProfileFailure | null>(null);

  function add(): void {
    editor = new SearchProfileForm(null);
    editorOpen = true;
  }

  function edit(profile: SearchProfile): void {
    editor = new SearchProfileForm(profile);
    editorOpen = true;
  }

  function test(profile: SearchProfile): void {
    const started = new SearchTestRun(profile);
    testRun = started;
    testOpen = true;
    void started.run();
  }

  async function confirmDelete(): Promise<void> {
    if (deleting === null) {
      return;
    }
    const outcome = await searchProfiles.delete(deleting.id);
    actionFailure = outcome.ok ? null : outcome.failure;
    deleting = null;
  }

  onMount(() => {
    void searchProfiles.ensureLoaded();
  });
</script>

<section class="flex flex-col gap-3" aria-labelledby="search-profiles-heading">
  <div class="flex flex-col gap-1">
    <h3 id="search-profiles-heading" class="text-sm font-medium">{m.chat_search_section_title()}</h3>
    <p class="text-sm text-muted-foreground">{m.chat_search_section_description()}</p>
  </div>
  {#if searchProfiles.status === 'loading'}
    <div class="flex flex-col gap-2" aria-busy="true">
      <Skeleton class="h-14 w-full" />
      <Skeleton class="h-14 w-full" />
    </div>
  {:else if searchProfiles.status === 'error'}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.shell_settings_load_failed_title()}</Alert.Title>
      <Alert.Description>
        {searchProfiles.failure === null ? '' : profileFailureMessage(searchProfiles.failure)}
      </Alert.Description>
      <Alert.Action>
        <Button size="sm" variant="outline" onclick={() => void searchProfiles.reload()}>{m.chat_try_again()}</Button>
      </Alert.Action>
    </Alert.Root>
  {:else if searchProfiles.profiles.length === 0}
    <Empty.Root class="border">
      <Empty.Header>
        <Empty.Title>{m.chat_search_empty_title()}</Empty.Title>
        <Empty.Description>{m.chat_search_empty_description()}</Empty.Description>
      </Empty.Header>
      <Empty.Content>
        <Button onclick={add}>
          <PlusIcon data-icon="inline-start" />
          {m.chat_search_add()}
        </Button>
      </Empty.Content>
    </Empty.Root>
  {:else}
    <ul class="flex flex-col divide-y rounded-md border" data-search-profiles="">
      {#each searchProfiles.profiles as profile (profile.id)}
        <li class="flex flex-wrap items-center gap-3 p-3">
          <div class="flex min-w-0 flex-1 flex-col">
            <span class="truncate font-medium" data-content>{profile.name}</span>
            <span class="truncate text-xs text-muted-foreground">
              <span data-content>{profile.model}</span>
              ·
              <span>{searchKindLabel(profile.kind)}</span>
            </span>
          </div>
          {#if profile.server !== null}
            <Badge variant="outline">{serverLabel(profile.server)}</Badge>
          {/if}
          <Button size="sm" variant="outline" onclick={() => test(profile)}>{m.chat_providers_test()}</Button>
          <DropdownMenu.Root>
            <DropdownMenu.Trigger>
              {#snippet child({ props })}
                <Button
                  {...props}
                  aria-label={m.chat_providers_menu({ name: profile.name })}
                  size="icon-sm"
                  variant="ghost"
                >
                  <EllipsisIcon />
                </Button>
              {/snippet}
            </DropdownMenu.Trigger>
            <DropdownMenu.Content align="end">
              <DropdownMenu.Group>
                <DropdownMenu.Item onSelect={() => edit(profile)}>{m.chat_providers_edit()}</DropdownMenu.Item>
              </DropdownMenu.Group>
              <DropdownMenu.Separator />
              <DropdownMenu.Group>
                <DropdownMenu.Item variant="destructive" onSelect={() => (deleting = profile)}>
                  {m.chat_delete()}
                </DropdownMenu.Item>
              </DropdownMenu.Group>
            </DropdownMenu.Content>
          </DropdownMenu.Root>
        </li>
      {/each}
    </ul>
    <div>
      <Button variant="outline" onclick={add}>
        <PlusIcon data-icon="inline-start" />
        {m.chat_search_add()}
      </Button>
    </div>
  {/if}
  {#if actionFailure !== null}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Description>{profileFailureMessage(actionFailure)}</Alert.Description>
    </Alert.Root>
  {/if}
</section>

<SearchProfileDialog form={editor} bind:open={editorOpen} />
<SearchTestDialog run={testRun} bind:open={testOpen} />
<AlertDialog.Root
  bind:open={
    () => deleting !== null,
    (open) => {
      if (!open) {
        deleting = null;
      }
    }
  }
>
  <AlertDialog.Content>
    <AlertDialog.Header>
      <AlertDialog.Title>{m.chat_search_delete_title()}</AlertDialog.Title>
      <AlertDialog.Description>
        {m.chat_search_delete_description({ name: deleting?.name ?? '' })}
      </AlertDialog.Description>
    </AlertDialog.Header>
    <AlertDialog.Footer>
      <AlertDialog.Cancel>{m.chat_cancel()}</AlertDialog.Cancel>
      <Button variant="destructive" onclick={confirmDelete}>{m.chat_delete()}</Button>
    </AlertDialog.Footer>
  </AlertDialog.Content>
</AlertDialog.Root>
