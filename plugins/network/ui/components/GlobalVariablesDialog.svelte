<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';

  import type { VariableEntry } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { networkDialogs } from '../state/network-dialogs.svelte.js';
  import { networkLibrary } from '../state/network.svelte.js';
  import type { NetworkFailure } from '../types/network.js';
  import { networkFailureMessage } from '../utils/network-messages.js';

  import KeyValueEditor from './KeyValueEditor.svelte';

  let entries = $state<VariableEntry[]>([]);
  let saving = $state(false);
  let failure = $state<NetworkFailure | null>(null);

  function setEntries(next: VariableEntry[]): void {
    entries = next;
  }

  function handleOpenChange(nextOpen: boolean): void {
    if (!nextOpen && !saving) {
      networkDialogs.closeGlobalVariables();
    }
  }

  async function saveGlobals(): Promise<void> {
    saving = true;
    failure = null;
    const outcome = await networkLibrary.saveGlobalVariables(entries);

    saving = false;
    if (outcome.ok) {
      networkDialogs.closeGlobalVariables();
    } else {
      failure = outcome.failure;
    }
  }

  $effect(() => {
    if (networkDialogs.globalVariablesOpen) {
      entries = networkLibrary.globalVariables.map((entry) => ({ ...entry }));
      failure = null;
      saving = false;
    }
  });
</script>

<Dialog.Root open={networkDialogs.globalVariablesOpen} onOpenChange={handleOpenChange}>
  <Dialog.Content class="flex max-h-[calc(100vh-2rem)] flex-col overflow-hidden sm:max-w-2xl" showCloseButton={false}>
    <div class="flex min-h-0 flex-1 flex-col gap-4">
      <Dialog.Header class="shrink-0">
        <Dialog.Title>{m.network_globals_title()}</Dialog.Title>
        <Dialog.Description>{m.network_globals_description()}</Dialog.Description>
      </Dialog.Header>

      <div class="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto pe-1">
        {#if failure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Description>{networkFailureMessage(failure)}</Alert.Description>
          </Alert.Root>
        {/if}

        <KeyValueEditor
          disabled={saving}
          {entries}
          keyPlaceholder={m.network_kv_variable_key_placeholder()}
          kind="variable"
          valuePlaceholder={m.network_kv_variable_value_placeholder()}
          onChange={setEntries}
        />
      </div>

      <Dialog.Footer class="shrink-0">
        <Button disabled={saving} type="button" variant="outline" onclick={() => networkDialogs.closeGlobalVariables()}>
          {m.network_cancel()}
        </Button>
        <Button disabled={saving} type="button" onclick={() => void saveGlobals()}>
          {saving ? m.network_saving() : m.network_save()}
        </Button>
      </Dialog.Footer>
    </div>
  </Dialog.Content>
</Dialog.Root>
