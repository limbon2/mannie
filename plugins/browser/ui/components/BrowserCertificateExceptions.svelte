<script lang="ts">
  import { FieldHelp } from '@mannie/plugin-sdk/components';
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import * as AlertDialog from '@mannie/plugin-sdk/components/alert-dialog';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Empty from '@mannie/plugin-sdk/components/empty';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import * as Item from '@mannie/plugin-sdk/components/item';
  import { Skeleton } from '@mannie/plugin-sdk/components/skeleton';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import ShieldCheckIcon from '@mannie/plugin-sdk/icons/shield-check';
  import { call } from '@mannie/plugin-sdk/ipc';
  import { onMount } from 'svelte';

  import { type BrowserCertificateException, commands } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { browserFailureMessage } from '../utils/browser-messages.js';

  const id = $props.id();
  let heading = $state<HTMLElement | null>(null);
  let failure = $state<string | null>(null);
  let saveFailure = $state<string | null>(null);
  let adding = $state(false);
  let hostnameInput = $state<HTMLInputElement | null>(null);
  let addTrigger = $state<HTMLButtonElement | null>(null);
  let notice = $state('');
  let saved = $state.raw<BrowserCertificateException[]>([]);
  let loading = $state(true);
  let listFailure = $state<string | null>(null);
  let busy = $state(false);

  async function load(showLoading = true): Promise<void> {
    loading = showLoading;
    listFailure = null;
    try {
      const outcome = await call(() => commands.browserSavedCertificates());
      if (outcome.ok) {
        saved = outcome.data;
      } else {
        listFailure = browserFailureMessage(outcome.failure);
      }
    } finally {
      loading = false;
    }
  }

  function focusHostname(event: Event): void {
    if (hostnameInput !== null) {
      event.preventDefault();
      hostnameInput.focus();
    }
  }

  async function save(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const form = event.currentTarget;
    const canSave = form instanceof HTMLFormElement && !busy;
    if (!canSave) {
      return;
    }
    const data = new FormData(form);
    const hostname = data.get('hostname');
    const fingerprint = data.get('fingerprint');
    const hasFields = typeof hostname === 'string' && typeof fingerprint === 'string';
    if (!hasFields) {
      return;
    }
    busy = true;
    saveFailure = null;
    notice = '';
    try {
      const outcome = await call(() => commands.browserSaveCertificate({ fingerprint, hostname }));
      if (outcome.ok) {
        form.reset();
        notice = m.browser_certificate_trust_saved();
        await load(false);
        adding = false;
      } else {
        saveFailure = browserFailureMessage(outcome.failure);
      }
    } finally {
      busy = false;
    }
  }

  async function forget({ fingerprint, hostname }: BrowserCertificateException): Promise<void> {
    if (busy) {
      return;
    }
    busy = true;
    failure = null;
    notice = '';
    try {
      const outcome = await call(() => commands.browserForgetCertificate({ fingerprint, hostname }));
      if (outcome.ok) {
        notice = m.browser_certificate_forgotten({ hostname });
        heading?.focus();
        await load(false);
      } else {
        failure = browserFailureMessage(outcome.failure);
      }
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    void load();
  });
</script>

<section class="flex min-w-0 flex-col gap-4" aria-labelledby={id} data-browser-trusted-hosts="">
  <h3 bind:this={heading} {id} class="font-medium focus-visible:ring-2 focus-visible:ring-ring" tabindex="-1">
    {m.browser_certificate_exceptions()}
  </h3>
  {#if failure !== null}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.browser_certificate_change_failed()}</Alert.Title>
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
        <span class="sr-only" role="status">{m.browser_certificate_loading()}</span>
        <Skeleton class="h-24 w-full" />
      </div>
    {:else if listFailure !== null}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.browser_certificate_load_failed()}</Alert.Title>
        <Alert.Description>{listFailure}</Alert.Description>
        <Alert.Action>
          <Button disabled={busy} size="sm" variant="outline" onclick={() => void load()}>{m.browser_retry()}</Button>
        </Alert.Action>
      </Alert.Root>
    {:else if saved.length === 0}
      <Empty.Root class="border">
        <Empty.Header>
          <Empty.Media variant="icon"><ShieldCheckIcon /></Empty.Media>
          <Empty.Title>{m.browser_certificate_empty()}</Empty.Title>
          <Empty.Description>{m.browser_certificate_empty_description()}</Empty.Description>
        </Empty.Header>
        <Empty.Content>
          <Dialog.Trigger>
            {#snippet child({ props })}
              <Button {...props} data-browser-certificate-add="" disabled={busy} bind:ref={addTrigger}>
                {m.browser_certificate_add()}
              </Button>
            {/snippet}
          </Dialog.Trigger>
        </Empty.Content>
      </Empty.Root>
    {:else}
      <div>
        <Dialog.Trigger>
          {#snippet child({ props })}
            <Button {...props} data-browser-certificate-add="" disabled={busy} variant="outline" bind:ref={addTrigger}>
              {m.browser_certificate_add()}
            </Button>
          {/snippet}
        </Dialog.Trigger>
      </div>
      <Item.Group aria-labelledby={id}>
        {#each saved as exception (exception.hostname)}
          <Item.Root data-browser-saved-certificate="" role="listitem" variant="outline">
            <Item.Content class="min-w-0">
              <Item.Title class="wrap-anywhere" data-content dir="ltr">{exception.hostname}</Item.Title>
              <code class="text-xs wrap-anywhere text-muted-foreground" data-content dir="ltr"
                >{exception.fingerprint}</code
              >
            </Item.Content>
            <Item.Actions>
              <AlertDialog.Root>
                <AlertDialog.Trigger>
                  {#snippet child({ props })}
                    <Button
                      {...props}
                      aria-label={m.browser_certificate_forget_label({ hostname: exception.hostname })}
                      data-browser-certificate-forget=""
                      disabled={busy}
                      size="sm"
                      variant="outline"
                    >
                      {m.browser_certificate_forget()}
                    </Button>
                  {/snippet}
                </AlertDialog.Trigger>
                <AlertDialog.Content>
                  <AlertDialog.Header class="min-w-0">
                    <AlertDialog.Title class="wrap-anywhere">
                      {m.browser_certificate_forget_question({ hostname: exception.hostname })}
                    </AlertDialog.Title>
                    <AlertDialog.Description>{m.browser_certificate_forget_description()}</AlertDialog.Description>
                  </AlertDialog.Header>
                  <AlertDialog.Footer>
                    <AlertDialog.Cancel>{m.browser_cancel()}</AlertDialog.Cancel>
                    <AlertDialog.Action
                      data-browser-certificate-forget-confirm=""
                      disabled={busy}
                      variant="destructive"
                      onclick={() => void forget(exception)}
                    >
                      {m.browser_certificate_forget()}
                    </AlertDialog.Action>
                  </AlertDialog.Footer>
                </AlertDialog.Content>
              </AlertDialog.Root>
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
      onOpenAutoFocus={focusHostname}
    >
      <form class="flex flex-col gap-4" data-browser-certificate-dialog="" onsubmit={(event) => void save(event)}>
        <Dialog.Header>
          <Dialog.Title>{m.browser_certificate_add()}</Dialog.Title>
          <Dialog.Description>{m.browser_certificate_description()}</Dialog.Description>
        </Dialog.Header>
        {#if saveFailure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Title>{m.browser_certificate_change_failed()}</Alert.Title>
            <Alert.Description>{saveFailure}</Alert.Description>
          </Alert.Root>
        {/if}
        <Field.FieldGroup>
          <Field.Field>
            <div class="flex items-center gap-1">
              <Field.Label for={`${id}-hostname`}>{m.browser_certificate_hostname()}</Field.Label>
              <FieldHelp
                id={`${id}-hostname-help`}
                field={m.browser_certificate_hostname()}
                text={m.browser_certificate_hostname_description()}
              />
            </div>
            <Input
              id={`${id}-hostname`}
              name="hostname"
              aria-describedby={`${id}-hostname-help`}
              autocomplete="off"
              data-browser-certificate-hostname=""
              dir="ltr"
              maxlength={1024}
              placeholder={m.browser_certificate_hostname_placeholder()}
              required
              spellcheck={false}
              bind:ref={hostnameInput}
            />
          </Field.Field>
          <Field.Field>
            <div class="flex items-center gap-1">
              <Field.Label for={`${id}-fingerprint`}>{m.browser_certificate_fingerprint()}</Field.Label>
              <FieldHelp
                id={`${id}-fingerprint-help`}
                field={m.browser_certificate_fingerprint()}
                text={m.browser_certificate_fingerprint_description()}
              />
            </div>
            <Input
              id={`${id}-fingerprint`}
              name="fingerprint"
              aria-describedby={`${id}-fingerprint-help`}
              autocomplete="off"
              data-browser-certificate-fingerprint=""
              dir="ltr"
              maxlength={128}
              placeholder={m.browser_certificate_fingerprint_placeholder()}
              required
              spellcheck={false}
            />
          </Field.Field>
        </Field.FieldGroup>
        <Dialog.Footer>
          <Dialog.Close>
            {#snippet child({ props })}
              <Button {...props} type="button" variant="outline">{m.browser_cancel()}</Button>
            {/snippet}
          </Dialog.Close>
          <Button data-browser-certificate-save="" disabled={busy} type="submit">{m.browser_certificate_save()}</Button>
        </Dialog.Footer>
      </form>
    </Dialog.Content>
  </Dialog.Root>
  <span class="sr-only" role="status">{notice}</span>
</section>
