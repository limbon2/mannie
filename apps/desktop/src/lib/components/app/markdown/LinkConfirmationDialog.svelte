<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';

  import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { linkConfirmation } from '#lib/state/link-confirmation.svelte.js';
  import { linkFailureMessage } from '#lib/utils/link-messages.js';

  // Shows a link from model output as the core normalized it, and opens it
  // only when the owner confirms (ADR-0021, decision 4). The shell mounts it
  // once as an overlay.
  const confirmation = $derived(linkConfirmation.state);
</script>

<AlertDialog.Root
  bind:open={
    () => linkConfirmation.state.kind !== 'closed',
    (open) => {
      if (!open) {
        linkConfirmation.close();
      }
    }
  }
>
  <AlertDialog.Content>
    <AlertDialog.Header>
      <AlertDialog.Title>{m.markdown_link_title()}</AlertDialog.Title>
      <AlertDialog.Description>{m.markdown_link_description()}</AlertDialog.Description>
    </AlertDialog.Header>
    {#if confirmation.kind === 'checking'}
      <p class="flex items-center gap-2 text-sm text-muted-foreground">
        <Spinner aria-label={m.markdown_link_checking()} />
        <span>{m.markdown_link_checking()}</span>
      </p>
    {:else if confirmation.kind === 'confirm'}
      <div class="flex flex-col gap-2">
        <dl class="flex flex-col gap-1 text-sm">
          <dt class="text-muted-foreground">{m.markdown_link_host()}</dt>
          <dd class="font-semibold break-all" data-content>{confirmation.link.host}</dd>
          <dt class="text-muted-foreground">{m.markdown_link_address()}</dt>
          <dd class="rounded-md bg-muted p-2 font-mono text-xs break-all" data-content>{confirmation.link.url}</dd>
        </dl>
        {#if confirmation.failure}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Description>{linkFailureMessage(confirmation.failure)}</Alert.Description>
          </Alert.Root>
        {/if}
      </div>
    {:else if confirmation.kind === 'refused'}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Description>{linkFailureMessage(confirmation.failure)}</Alert.Description>
      </Alert.Root>
    {/if}
    <AlertDialog.Footer>
      <AlertDialog.Cancel>{m.markdown_link_cancel()}</AlertDialog.Cancel>
      {#if confirmation.kind === 'confirm'}
        <Button disabled={confirmation.opening} onclick={() => void linkConfirmation.open()}>
          {#if confirmation.opening}
            <Spinner aria-label={m.markdown_link_opening()} data-icon="inline-start" />
          {/if}
          {m.markdown_link_open()}
        </Button>
      {/if}
    </AlertDialog.Footer>
  </AlertDialog.Content>
</AlertDialog.Root>
