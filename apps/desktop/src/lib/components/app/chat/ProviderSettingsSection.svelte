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
  import type { ProviderProfile } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { ProviderProfileForm } from '#lib/state/provider-profile-form.svelte.js';
  import { providerProfiles } from '#lib/state/provider-profiles.svelte.js';
  import { ProviderTestRun } from '#lib/state/provider-test-run.svelte.js';
  import type { ProfileFailure } from '#lib/types/chat.js';
  import { profileFailureMessage } from '#lib/utils/chat-errors.js';
  import { interfaceLabel, serverLabel } from '#lib/utils/provider-labels.js';

  import ProviderProfileDialog from './ProviderProfileDialog.svelte';
  import ProviderTestDialog from './ProviderTestDialog.svelte';
  import SearchProfilesList from './SearchProfilesList.svelte';

  // The provider profiles (docs/features/chat/conversation-view.md, settings
  // sections): each with its model and interface, which one is the default,
  // and Edit, Test, Make default and Delete; below them, the search profiles
  // of embedding and rerank models (ADR-0047).
  let editorOpen = $state(false);
  let editor = $state<ProviderProfileForm | null>(null);
  let testOpen = $state(false);
  let testRun = $state<ProviderTestRun | null>(null);
  let deleting = $state<ProviderProfile | null>(null);
  let actionFailure = $state<ProfileFailure | null>(null);

  function add(): void {
    editor = new ProviderProfileForm(null);
    editorOpen = true;
  }

  function edit(profile: ProviderProfile): void {
    editor = new ProviderProfileForm(profile);
    editorOpen = true;
  }

  function test(profile: ProviderProfile): void {
    const started = new ProviderTestRun(profile);
    testRun = started;
    testOpen = true;
    void started.run();
  }

  async function makeDefault(profile: ProviderProfile): Promise<void> {
    const outcome = await providerProfiles.setDefault(profile.id);
    actionFailure = outcome.ok ? null : outcome.failure;
  }

  async function confirmDelete(): Promise<void> {
    if (deleting === null) {
      return;
    }
    const outcome = await providerProfiles.delete(deleting.id);
    actionFailure = outcome.ok ? null : outcome.failure;
    deleting = null;
  }

  onMount(() => {
    void providerProfiles.ensureLoaded();
  });
</script>

{#if providerProfiles.status === 'loading'}
  <div class="flex flex-col gap-2" aria-busy="true">
    <Skeleton class="h-14 w-full" />
    <Skeleton class="h-14 w-full" />
  </div>
{:else if providerProfiles.status === 'error'}
  <Alert.Root variant="destructive">
    <CircleAlertIcon />
    <Alert.Title>{m.shell_settings_load_failed_title()}</Alert.Title>
    <Alert.Description>
      {providerProfiles.failure === null ? '' : profileFailureMessage(providerProfiles.failure)}
    </Alert.Description>
    <Alert.Action>
      <Button size="sm" variant="outline" onclick={() => void providerProfiles.reload()}>{m.chat_try_again()}</Button>
    </Alert.Action>
  </Alert.Root>
{:else if providerProfiles.profiles.length === 0}
  <Empty.Root class="border">
    <Empty.Header>
      <Empty.Title>{m.chat_providers_empty_title()}</Empty.Title>
      <Empty.Description>{m.chat_providers_empty_description()}</Empty.Description>
    </Empty.Header>
    <Empty.Content>
      <Button onclick={add}>
        <PlusIcon data-icon="inline-start" />
        {m.chat_providers_add()}
      </Button>
    </Empty.Content>
  </Empty.Root>
{:else}
  <ul class="flex flex-col divide-y rounded-md border">
    {#each providerProfiles.profiles as profile (profile.id)}
      <li class="flex flex-wrap items-center gap-3 p-3">
        <div class="flex min-w-0 flex-1 flex-col">
          <span class="truncate font-medium" data-content>{profile.name}</span>
          <span class="truncate text-xs text-muted-foreground">
            <span data-content>{profile.model}</span>
            ·
            <span>{interfaceLabel(profile.interface)}</span>
          </span>
        </div>
        {#if profile.isDefault}
          <Badge variant="secondary">{m.chat_providers_default()}</Badge>
        {/if}
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
              <DropdownMenu.Item disabled={profile.isDefault} onSelect={() => void makeDefault(profile)}>
                {m.chat_providers_make_default()}
              </DropdownMenu.Item>
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
      {m.chat_providers_add()}
    </Button>
  </div>
{/if}
{#if actionFailure !== null}
  <Alert.Root variant="destructive">
    <CircleAlertIcon />
    <Alert.Description>{profileFailureMessage(actionFailure)}</Alert.Description>
  </Alert.Root>
{/if}
<SearchProfilesList />

<ProviderProfileDialog form={editor} bind:open={editorOpen} />
<ProviderTestDialog run={testRun} bind:open={testOpen} />
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
      <AlertDialog.Title>{m.chat_providers_delete_title()}</AlertDialog.Title>
      <AlertDialog.Description>
        {m.chat_providers_delete_description({ name: deleting?.name ?? '' })}
      </AlertDialog.Description>
    </AlertDialog.Header>
    <AlertDialog.Footer>
      <AlertDialog.Cancel>{m.chat_cancel()}</AlertDialog.Cancel>
      <Button variant="destructive" onclick={confirmDelete}>{m.chat_delete()}</Button>
    </AlertDialog.Footer>
  </AlertDialog.Content>
</AlertDialog.Root>
