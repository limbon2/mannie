<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import * as Item from '@mannie/plugin-sdk/components/item';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import { Switch } from '@mannie/plugin-sdk/components/switch';
  import { Textarea } from '@mannie/plugin-sdk/components/textarea';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';

  import type { ConfiguredServerView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { mcpActions } from '../state/mcp-actions.svelte.js';
  import { entryRefusalMessage, mcpFailureMessage } from '../utils/mcp-messages.js';

  // Paste JSON (plugins/mcp/docs/README.md, registering a server): the
  // owner pastes another client's configuration, Mannie reads it without
  // running anything and lists its servers with what each would run and the
  // names of its secrets, never their values, and the owner chooses the ones
  // to add. A server that cannot be added says why. Only the part between
  // the title and the buttons scrolls, so both stay in view.
  const request = $derived(mcpActions.paste);
  const isBusy = $derived(request !== null && (request.reading || request.adding));
  /** Whether the dialog shows the servers that were read and not added yet. */
  const showsPreview = $derived(request?.preview !== null && request?.outcome === null);
  /** The servers that the dialog shows, or `null`. */
  const preview = $derived(showsPreview ? (request?.preview ?? null) : null);

  function choose(server: ConfiguredServerView, chosen: boolean): void {
    if (request === null) {
      return;
    }
    const others = request.chosen.filter((key) => key !== server.key);
    request.chosen = chosen ? [...others, server.key] : others;
  }

  function read(event: SubmitEvent): void {
    event.preventDefault();
    const isClosed = request === null;
    if (isClosed || isBusy) {
      return;
    }
    void mcpActions.readPaste(request);
  }
</script>

<Dialog.Root bind:open={mcpActions.pasteOpen}>
  <Dialog.Content class="flex max-h-[90vh] flex-col sm:max-w-2xl" showCloseButton={false}>
    {#if request !== null}
      <div class="flex min-h-0 flex-col gap-4" data-mcp-paste-dialog="">
        <Dialog.Header>
          <Dialog.Title>{m.mcp_paste_title()}</Dialog.Title>
          <Dialog.Description>{m.mcp_paste_description()}</Dialog.Description>
        </Dialog.Header>
        <div class="relative -mx-4 flex min-h-0 flex-col gap-4 overflow-y-auto px-4 py-1">
          <form class="flex flex-col gap-2" onsubmit={read}>
            <Field.Field>
              <Field.Label for="mcp-paste-text">{m.mcp_paste_field()}</Field.Label>
              <Textarea
                id="mcp-paste-text"
                class="max-h-60 min-h-32 font-mono"
                dir="ltr"
                disabled={isBusy}
                spellcheck="false"
                bind:value={request.text}
              />
            </Field.Field>
            <div>
              <Button
                data-mcp-paste-read=""
                disabled={isBusy || request.text.trim() === ''}
                type="submit"
                variant="outline"
              >
                {m.mcp_paste_read()}
              </Button>
            </div>
          </form>
          <div class="text-sm" role="status">
            {#if request.reading}
              <p class="flex items-center gap-2 text-muted-foreground">
                <Spinner aria-hidden="true" />
                <span>{m.mcp_paste_reading()}</span>
              </p>
            {:else if request.adding}
              <p class="flex items-center gap-2 text-muted-foreground">
                <Spinner aria-hidden="true" />
                <span>{m.mcp_paste_adding()}</span>
              </p>
            {:else if request.outcome !== null}
              <p data-mcp-paste-added="">{m.mcp_paste_added({ count: request.outcome.added.length })}</p>
              {#each request.outcome.skipped as skip (skip.key)}
                <p class="text-destructive">
                  {m.mcp_paste_skipped({ key: skip.key, reason: entryRefusalMessage(skip.reason) })}
                </p>
              {/each}
            {/if}
          </div>
          {#if preview !== null}
            <Item.Group aria-label={m.mcp_paste_servers_label()}>
              {#each preview as server (server.key)}
                <Item.Root data-mcp-paste-server={server.key} role="listitem" size="sm" variant="outline">
                  <Item.Content class="min-w-0 gap-1">
                    <Item.Title class="min-w-0 wrap-anywhere" data-content>{server.key}</Item.Title>
                    {#if server.program !== null}
                      <p class="font-mono text-sm wrap-anywhere text-muted-foreground" data-content dir="ltr">
                        {m.mcp_paste_server_program({ program: [server.program, ...server.arguments].join(' ') })}
                      </p>
                    {:else if server.url !== null}
                      <p class="font-mono text-sm wrap-anywhere text-muted-foreground" data-content dir="ltr">
                        {m.mcp_paste_server_url({ url: server.url })}
                      </p>
                    {/if}
                    {#if server.secretNames.length > 0}
                      <p class="text-sm text-muted-foreground">
                        {m.mcp_paste_secrets({ names: server.secretNames.join(', ') })}
                      </p>
                    {/if}
                    {#if server.missingSecrets.length > 0}
                      <p class="text-sm">{m.mcp_paste_missing({ names: server.missingSecrets.join(', ') })}</p>
                    {/if}
                    {#if server.disabled}
                      <p class="text-sm">{m.mcp_paste_disabled()}</p>
                    {/if}
                    {#if server.refusal !== null}
                      <p class="text-sm text-destructive" data-mcp-paste-refusal={server.refusal}>
                        {entryRefusalMessage(server.refusal)}
                      </p>
                    {/if}
                  </Item.Content>
                  <Item.Actions class="self-start">
                    <Switch
                      aria-label={m.mcp_paste_choose({ server: server.key })}
                      checked={request.chosen.includes(server.key)}
                      disabled={server.refusal !== null || isBusy}
                      onCheckedChange={(checked: boolean) => choose(server, checked)}
                    />
                  </Item.Actions>
                </Item.Root>
              {/each}
            </Item.Group>
          {/if}
        </div>
        {#if request.failure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Title>
              {request.preview === null ? m.mcp_paste_failed_title() : m.mcp_paste_add_failed_title()}
            </Alert.Title>
            <Alert.Description>{mcpFailureMessage(request.failure)}</Alert.Description>
          </Alert.Root>
        {/if}
        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={() => (mcpActions.pasteOpen = false)}>
            {request.outcome === null ? m.mcp_cancel() : m.mcp_close()}
          </Button>
          {#if showsPreview}
            <Button
              data-mcp-paste-add=""
              disabled={isBusy || request.chosen.length === 0}
              type="button"
              onclick={() => void mcpActions.importChosen(request)}
            >
              {m.mcp_paste_add({ count: request.chosen.length })}
            </Button>
          {/if}
        </Dialog.Footer>
      </div>
    {/if}
  </Dialog.Content>
</Dialog.Root>
