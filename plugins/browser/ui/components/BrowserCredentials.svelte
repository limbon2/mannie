<script lang="ts">
  import { FieldHelp } from '@mannie/plugin-sdk/components';
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Empty from '@mannie/plugin-sdk/components/empty';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import * as Item from '@mannie/plugin-sdk/components/item';
  import { Skeleton } from '@mannie/plugin-sdk/components/skeleton';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import KeyRoundIcon from '@mannie/plugin-sdk/icons/key-round';
  import { call } from '@mannie/plugin-sdk/ipc';
  import { onMount } from 'svelte';

  import { type BrowserCredentialRef, commands } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { browserFailureMessage } from '../utils/browser-messages.js';

  const id = $props.id();
  let heading = $state<HTMLElement | null>(null);
  let credentials = $state.raw<BrowserCredentialRef[]>([]);
  let loading = $state(true);
  let busy = $state(false);
  let listFailure = $state<string | null>(null);
  let failure = $state<string | null>(null);
  let saveFailure = $state<string | null>(null);
  let adding = $state(false);
  let originInput = $state<HTMLInputElement | null>(null);
  let addTrigger = $state<HTMLButtonElement | null>(null);

  async function load(showLoading = true): Promise<void> {
    loading = showLoading;
    listFailure = null;
    try {
      const outcome = await call(() => commands.browserSiteCredentials());
      if (outcome.ok) {
        credentials = outcome.data;
      } else {
        listFailure = browserFailureMessage(outcome.failure);
      }
    } finally {
      loading = false;
    }
  }

  function focusOrigin(event: Event): void {
    if (originInput !== null) {
      event.preventDefault();
      originInput.focus();
    }
  }

  async function add(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const form = event.currentTarget;
    const canSave = form instanceof HTMLFormElement && !busy;
    if (!canSave) {
      return;
    }
    const origin = form.elements.namedItem('origin');
    const label = form.elements.namedItem('label');
    const secret = form.elements.namedItem('value');
    const hasFields =
      origin instanceof HTMLInputElement && label instanceof HTMLInputElement && secret instanceof HTMLInputElement;
    if (!hasFields) {
      return;
    }
    busy = true;
    saveFailure = null;
    const value = secret.value;
    secret.value = '';
    try {
      const outcome = await call(() =>
        commands.browserAddSiteCredential({ origin: origin.value, label: label.value, value }),
      );
      if (outcome.ok) {
        form.reset();
        await load(false);
        adding = false;
      } else {
        saveFailure = browserFailureMessage(outcome.failure);
      }
    } finally {
      busy = false;
    }
  }

  async function remove(reference: string): Promise<void> {
    if (busy) {
      return;
    }
    busy = true;
    failure = null;
    try {
      const outcome = await call(() => commands.browserRemoveSiteCredential({ reference }));
      if (!outcome.ok) {
        failure = browserFailureMessage(outcome.failure);
      }
      heading?.focus();
      await load(false);
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    void load();
  });
</script>

<section class="flex min-w-0 flex-col gap-4" aria-labelledby={id} data-browser-site-credentials="">
  <h3 bind:this={heading} {id} class="font-medium focus-visible:ring-2 focus-visible:ring-ring" tabindex="-1">
    {m.browser_credentials()}
  </h3>
  {#if failure !== null}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.browser_credential_change_failed()}</Alert.Title>
      <Alert.Description>{failure}</Alert.Description>
    </Alert.Root>
  {/if}
  <Dialog.Root
    bind:open={adding}
    onOpenChange={(open) => {
      if (open) {
        saveFailure = null;
      }
    }}
  >
    {#if loading}
      <div class="flex flex-col gap-3" aria-busy="true">
        <span class="sr-only" role="status">{m.browser_credential_loading()}</span>
        <Skeleton class="h-24 w-full" />
      </div>
    {:else if listFailure !== null}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.browser_credential_load_failed()}</Alert.Title>
        <Alert.Description>{listFailure}</Alert.Description>
        <Alert.Action>
          <Button disabled={busy} size="sm" variant="outline" onclick={() => void load()}>{m.browser_retry()}</Button>
        </Alert.Action>
      </Alert.Root>
    {:else if credentials.length === 0}
      <Empty.Root class="border">
        <Empty.Header>
          <Empty.Media variant="icon"><KeyRoundIcon /></Empty.Media>
          <Empty.Title>{m.browser_credentials_empty()}</Empty.Title>
          <Empty.Description>{m.browser_credential_empty_description()}</Empty.Description>
        </Empty.Header>
        <Empty.Content>
          <Dialog.Trigger>
            {#snippet child({ props })}
              <Button {...props} data-browser-credential-add="" disabled={busy} bind:ref={addTrigger}>
                {m.browser_credential_add()}
              </Button>
            {/snippet}
          </Dialog.Trigger>
        </Empty.Content>
      </Empty.Root>
    {:else}
      <div>
        <Dialog.Trigger>
          {#snippet child({ props })}
            <Button {...props} data-browser-credential-add="" disabled={busy} variant="outline" bind:ref={addTrigger}>
              {m.browser_credential_add()}
            </Button>
          {/snippet}
        </Dialog.Trigger>
      </div>
      <Item.Group aria-labelledby={id}>
        {#each credentials as credential (credential.reference)}
          <Item.Root role="listitem" variant="outline">
            <Item.Content class="min-w-0">
              <Item.Title class="wrap-anywhere" data-content dir="auto">{credential.label}</Item.Title>
              <p class="font-mono text-xs wrap-anywhere text-muted-foreground" data-content dir="ltr">
                {credential.origin}
              </p>
            </Item.Content>
            <Item.Actions>
              <Button
                aria-label={m.browser_credential_remove({ label: credential.label })}
                data-browser-credential-remove=""
                disabled={busy}
                size="sm"
                variant="outline"
                onclick={() => void remove(credential.reference)}
              >
                {m.browser_credential_remove_action()}
              </Button>
            </Item.Actions>
          </Item.Root>
        {/each}
      </Item.Group>
    {/if}
    <Dialog.Content
      class="sm:max-w-lg"
      showCloseButton={false}
      onCloseAutoFocus={(event: Event) => {
        event.preventDefault();
        (addTrigger?.disabled === false ? addTrigger : heading)?.focus();
      }}
      onOpenAutoFocus={focusOrigin}
    >
      <form class="flex flex-col gap-4" data-browser-credential-dialog="" onsubmit={(event) => void add(event)}>
        <Dialog.Header>
          <Dialog.Title>{m.browser_credential_add()}</Dialog.Title>
          <Dialog.Description>{m.browser_credentials_description()}</Dialog.Description>
        </Dialog.Header>
        {#if saveFailure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Title>{m.browser_credential_change_failed()}</Alert.Title>
            <Alert.Description>{saveFailure}</Alert.Description>
          </Alert.Root>
        {/if}
        <Field.FieldGroup>
          <Field.Field>
            <div class="flex items-center gap-1">
              <Field.Label for={`${id}-origin`}>{m.browser_credential_site()}</Field.Label>
              <FieldHelp
                id={`${id}-origin-help`}
                field={m.browser_credential_site()}
                text={m.browser_credential_site_description()}
              />
            </div>
            <Input
              id={`${id}-origin`}
              name="origin"
              aria-describedby={`${id}-origin-help`}
              dir="ltr"
              placeholder={m.browser_credential_site_placeholder()}
              required
              type="url"
              bind:ref={originInput}
            />
          </Field.Field>
          <Field.Field>
            <div class="flex items-center gap-1">
              <Field.Label for={`${id}-label`}>{m.browser_credential_label()}</Field.Label>
              <FieldHelp
                id={`${id}-label-help`}
                field={m.browser_credential_label()}
                text={m.browser_credential_label_description()}
              />
            </div>
            <Input
              id={`${id}-label`}
              name="label"
              aria-describedby={`${id}-label-help`}
              placeholder={m.browser_credential_label_placeholder()}
              required
            />
          </Field.Field>
          <Field.Field>
            <div class="flex items-center gap-1">
              <Field.Label for={`${id}-secret`}>{m.browser_credential_value()}</Field.Label>
              <FieldHelp
                id={`${id}-secret-help`}
                field={m.browser_credential_value()}
                text={m.browser_credential_value_description()}
              />
            </div>
            <Input
              id={`${id}-secret`}
              name="value"
              aria-describedby={`${id}-secret-help`}
              autocomplete="off"
              placeholder={m.browser_credential_value_placeholder()}
              required
              type="password"
            />
          </Field.Field>
        </Field.FieldGroup>
        <Dialog.Footer>
          <Dialog.Close>
            {#snippet child({ props })}
              <Button {...props} type="button" variant="outline">{m.browser_cancel()}</Button>
            {/snippet}
          </Dialog.Close>
          <Button data-browser-credential-save="" disabled={busy} type="submit">{m.browser_credential_save()}</Button>
        </Dialog.Footer>
      </form>
    </Dialog.Content>
  </Dialog.Root>
</section>
