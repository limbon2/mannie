<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import XIcon from '@mannie/plugin-sdk/icons/x';

  import { m } from '../../generated/messages.js';
  import type { RepositoryPanel } from '../../state/git-repository.svelte.js';
  import { gitFailureMessage } from '../../utils/git-messages.js';
  import DiffView from '../cards/DiffView.svelte';

  // Split-pane diff viewer for a chosen stash entry inside the Branches tab.
  interface Props {
    readonly panel: RepositoryPanel;
  }

  let { panel }: Props = $props();

  const stash = $derived(panel.stashView.value);
  const selectorLabel = $derived.by(() => {
    if (stash !== null) {
      return stash.entry.selector;
    }
    const index = panel.chosenStashIndex ?? 0;
    return `stash@{${index}}`;
  });
  const isLoadingInitial = $derived(panel.stashView.status === 'loading' && stash === null);
  const loadFailure = $derived(panel.stashView.status === 'error' ? panel.stashView.failure : null);
</script>

<div class="flex h-full min-h-0 flex-col overflow-hidden">
  <div class="flex items-center justify-between gap-2 border-b bg-muted/30 px-2.5 py-1.5 text-xs">
    <div class="flex min-w-0 items-center gap-1.5">
      <Badge class="font-mono" variant="outline">{selectorLabel}</Badge>
      <span class="min-w-0 truncate font-medium">{stash?.entry.action ?? ''}</span>
    </div>

    <Button
      class="size-6 shrink-0"
      aria-label={m.git_dialog_close()}
      size="icon"
      variant="ghost"
      onclick={() => panel.closeStash()}
    >
      <XIcon class="size-3.5" aria-hidden="true" />
    </Button>
  </div>

  <div class="min-h-0 flex-1 overflow-y-auto p-2.5">
    {#if isLoadingInitial}
      <div class="flex items-center gap-2 py-4 text-xs text-muted-foreground" aria-busy="true">
        <Spinner aria-label={m.git_panel_loading()} />
        <span>{m.git_panel_loading()}</span>
      </div>
    {:else if loadFailure !== null}
      <Alert.Root variant="destructive">
        <Alert.Description class="text-xs">{gitFailureMessage(loadFailure)}</Alert.Description>
      </Alert.Root>
    {:else if stash !== null}
      <DiffView files={stash.files} mode={panel.diffMode} open={stash.files.length === 1} patches={stash.patches} />
    {/if}
  </div>
</div>
