<script lang="ts">
  import { FieldHelp } from '@mannie/plugin-sdk/components';
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';

  import { m } from '../generated/messages.js';
  import { workersActions } from '../state/workers-actions.svelte.js';
  import { workersFailureMessage } from '../utils/workers-messages.js';

  // Pairs a worker (plugins/workers/docs/worker-activity.md, pairing a
  // worker): the host's address and the code that `mannie-worker pair`
  // printed on it, and, if the owner likes, a name for the worker. "Pair"
  // pairs and closes the dialog; while it pairs, a status region says so,
  // and a refusal stays in the dialog with what the owner can do.
  let addressInput: HTMLInputElement | null = $state(null);

  const request = $derived(workersActions.pair);

  /** The dialog opens in the Address field, which its help button precedes in the page. */
  function focusAddress(event: Event): void {
    if (addressInput !== null) {
      event.preventDefault();
      addressInput.focus();
    }
  }

  function submit(event: SubmitEvent): void {
    event.preventDefault();
    if (request === null) {
      return;
    }
    if (!request.pairing) {
      void workersActions.confirmPair(request);
    }
  }
</script>

<Dialog.Root bind:open={workersActions.pairOpen}>
  <Dialog.Content showCloseButton={false} onOpenAutoFocus={focusAddress}>
    {#if request !== null}
      <form class="flex flex-col gap-4" data-pair-worker-dialog="" onsubmit={submit}>
        <Dialog.Header>
          <Dialog.Title>{m.workers_pair_title()}</Dialog.Title>
          <Dialog.Description>{m.workers_pair_description()}</Dialog.Description>
        </Dialog.Header>
        <Field.Group>
          <Field.Field>
            <div class="flex items-center gap-1">
              <Field.Label for="pair-worker-address">{m.workers_pair_address()}</Field.Label>
              <FieldHelp
                id="pair-worker-address-help"
                field={m.workers_pair_address()}
                text={m.workers_pair_address_description()}
              />
            </div>
            <Input
              id="pair-worker-address"
              class="font-mono"
              aria-describedby="pair-worker-address-help"
              autocomplete="off"
              disabled={request.pairing}
              placeholder={m.workers_pair_address_placeholder()}
              required
              spellcheck="false"
              bind:ref={addressInput}
              bind:value={request.address}
            />
          </Field.Field>
          <Field.Field>
            <div class="flex items-center gap-1">
              <Field.Label for="pair-worker-code">{m.workers_pair_code()}</Field.Label>
              <FieldHelp
                id="pair-worker-code-help"
                field={m.workers_pair_code()}
                text={m.workers_pair_code_description()}
              />
            </div>
            <Input
              id="pair-worker-code"
              class="font-mono uppercase"
              aria-describedby="pair-worker-code-help"
              autocapitalize="characters"
              autocomplete="off"
              disabled={request.pairing}
              required
              spellcheck="false"
              bind:value={request.code}
            />
          </Field.Field>
          <Field.Field>
            <div class="flex items-center gap-1">
              <Field.Label for="pair-worker-name">{m.workers_pair_name()}</Field.Label>
              <FieldHelp
                id="pair-worker-name-help"
                field={m.workers_pair_name()}
                text={m.workers_pair_name_description()}
              />
            </div>
            <Input
              id="pair-worker-name"
              aria-describedby="pair-worker-name-help"
              autocomplete="off"
              disabled={request.pairing}
              spellcheck="false"
              bind:value={request.name}
            />
          </Field.Field>
        </Field.Group>
        <div role="status">
          {#if request.pairing}
            <p class="flex items-center gap-2 text-sm text-muted-foreground">
              <Spinner aria-hidden="true" />
              <span>{m.workers_pair_pending()}</span>
            </p>
          {/if}
        </div>
        {#if request.failure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Title>{m.workers_pair_failed_title()}</Alert.Title>
            <Alert.Description>{workersFailureMessage(request.failure)}</Alert.Description>
          </Alert.Root>
        {/if}
        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={() => (workersActions.pairOpen = false)}>
            {m.workers_cancel()}
          </Button>
          <Button data-pair-worker-submit="" disabled={request.pairing} type="submit">
            {m.workers_pair_submit()}
          </Button>
        </Dialog.Footer>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>
