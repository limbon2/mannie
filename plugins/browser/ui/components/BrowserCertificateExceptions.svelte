<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import * as AlertDialog from '@mannie/plugin-sdk/components/alert-dialog';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import { call } from '@mannie/plugin-sdk/ipc';

  import { type BrowserCertificateException, commands } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { browserFailureMessage } from '../utils/browser-messages.js';

  interface Props {
    busy: boolean;
    exceptions: BrowserCertificateException[];
    sessionId: string | null;
    trust: (
      sessionId: string,
      hostname: string,
      fingerprint: string,
      showFailure: (message: string) => void,
    ) => Promise<boolean>;
  }

  let { busy, exceptions, sessionId, trust }: Props = $props();
  const id = $props.id();
  let open = $state(false);
  let openedSessionId = $state<string | null>(null);
  let failure = $state<string | null>(null);
  let notice = $state('');
  let savedExceptions = $state.raw<BrowserCertificateException[] | null>(null);
  let loading = $state(false);
  let listFailure = $state<string | null>(null);
  let changing = $state(false);
  let pendingForget = $state.raw<BrowserCertificateException | null>(null);
  const disabled = $derived(busy || changing);
  const canTrustSession = $derived(openedSessionId !== null && openedSessionId === sessionId);
  const sessionTrustDisabled = $derived(disabled || !canTrustSession);
  const showSessionExceptions = $derived(canTrustSession && exceptions.length > 0);

  function setOpen(next: boolean): void {
    if (changing) {
      return;
    }
    open = next;
    if (next) {
      openedSessionId = sessionId;
      pendingForget = null;
      failure = null;
      notice = '';
      void loadSavedExceptions();
    }
  }

  async function loadSavedExceptions(): Promise<void> {
    if (loading) {
      return;
    }
    loading = true;
    listFailure = null;
    try {
      const outcome = await call(() => commands.browserSavedCertificates());
      if (outcome.ok) {
        savedExceptions = outcome.data;
      } else {
        listFailure = browserFailureMessage(outcome.failure);
      }
    } finally {
      loading = false;
    }
  }

  async function confirm(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const form = event.currentTarget;
    const canSubmit = form instanceof HTMLFormElement && !disabled;
    if (!canSubmit) {
      return;
    }
    const data = new FormData(form);
    const hostname = data.get('hostname');
    const fingerprint = data.get('fingerprint');
    const hasFields = typeof hostname === 'string' && typeof fingerprint === 'string';
    if (!hasFields) {
      return;
    }
    const saved = event.submitter instanceof HTMLButtonElement && event.submitter.value === 'saved';
    const confirmedSession = canTrustSession ? openedSessionId : null;
    failure = null;
    changing = true;
    try {
      let accepted = false;
      if (saved) {
        accepted = await saveCertificate(hostname, fingerprint);
      } else if (confirmedSession !== null) {
        accepted = await trust(confirmedSession, hostname, fingerprint, (message) => {
          failure = message;
        });
      } else {
        failure = m.browser_error_session_missing();
      }
      if (accepted) {
        form.reset();
        open = false;
        notice = saved ? m.browser_certificate_trust_saved() : m.browser_certificate_saved();
      }
    } finally {
      changing = false;
    }
  }

  async function saveCertificate(hostname: string, fingerprint: string): Promise<boolean> {
    const outcome = await call(() => commands.browserSaveCertificate({ fingerprint, hostname }));
    if (!outcome.ok) {
      failure = browserFailureMessage(outcome.failure);
    }
    return outcome.ok;
  }

  async function forgetCertificate({ fingerprint, hostname }: BrowserCertificateException): Promise<void> {
    if (disabled) {
      return;
    }
    pendingForget = null;
    failure = null;
    changing = true;
    try {
      const outcome = await call(() => commands.browserForgetCertificate({ fingerprint, hostname }));
      if (outcome.ok) {
        open = false;
        notice = m.browser_certificate_forgotten({ hostname });
      } else {
        failure = browserFailureMessage(outcome.failure);
      }
    } finally {
      changing = false;
    }
  }
</script>

<Dialog.Root bind:open={() => open, setOpen}>
  <Dialog.Trigger>
    {#snippet child({ props })}
      <Button {...props} data-browser-certificates="" {disabled} size="sm" variant="outline">
        {m.browser_certificate_trust()}
      </Button>
    {/snippet}
  </Dialog.Trigger>
  <Dialog.Content class="grid-cols-[minmax(0,1fr)] sm:max-w-lg" showCloseButton={false}>
    <Dialog.Header>
      <Dialog.Title>{m.browser_certificate_exceptions()}</Dialog.Title>
      <Dialog.Description>{m.browser_certificate_description()}</Dialog.Description>
    </Dialog.Header>
    {#if failure !== null}
      <Alert.Root variant="destructive">
        <Alert.Title>{m.browser_error_title()}</Alert.Title>
        <Alert.Description>{failure}</Alert.Description>
      </Alert.Root>
    {/if}
    <section class="flex flex-col gap-2" aria-labelledby={`${id}-saved`}>
      <h3 id={`${id}-saved`} class="font-medium">{m.browser_certificate_saved_list()}</h3>
      {#if loading}
        <p class="text-muted-foreground" role="status">{m.browser_certificate_loading()}</p>
      {:else if listFailure !== null}
        <Alert.Root variant="destructive">
          <Alert.Title>{m.browser_error_title()}</Alert.Title>
          <Alert.Description>{listFailure}</Alert.Description>
        </Alert.Root>
        <Button class="self-start" {disabled} size="sm" variant="outline" onclick={() => void loadSavedExceptions()}>
          {m.browser_retry()}
        </Button>
      {:else if savedExceptions?.length === 0}
        <p class="text-muted-foreground">{m.browser_certificate_empty()}</p>
      {:else if savedExceptions !== null}
        <ul class="flex flex-col gap-2">
          {#each savedExceptions as exception (exception.hostname)}
            <li class="flex items-center justify-between gap-2" data-browser-saved-certificate="">
              <div class="flex min-w-0 flex-col gap-1">
                <span class="wrap-anywhere" data-content dir="auto">{exception.hostname}</span>
                <code class="text-xs wrap-anywhere text-muted-foreground" data-content>{exception.fingerprint}</code>
              </div>
              <AlertDialog.Root
                bind:open={() => pendingForget === exception, (next) => (pendingForget = next ? exception : null)}
              >
                <AlertDialog.Trigger>
                  {#snippet child({ props })}
                    <Button
                      {...props}
                      aria-label={m.browser_certificate_forget_label({ hostname: exception.hostname })}
                      data-browser-certificate-forget=""
                      {disabled}
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
                      {disabled}
                      variant="destructive"
                      onclick={() => void forgetCertificate(exception)}
                    >
                      {m.browser_certificate_forget()}
                    </AlertDialog.Action>
                  </AlertDialog.Footer>
                </AlertDialog.Content>
              </AlertDialog.Root>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
    {#if showSessionExceptions}
      <section class="flex flex-col gap-2" aria-labelledby={`${id}-session`}>
        <h3 id={`${id}-session`} class="font-medium">{m.browser_certificate_session_list()}</h3>
        <ul class="flex flex-col gap-2">
          {#each exceptions as exception (exception.hostname)}
            <li class="flex flex-col gap-1" data-browser-certificate-exception="">
              <span class="wrap-anywhere" data-content dir="auto">{exception.hostname}</span>
              <code class="text-xs wrap-anywhere text-muted-foreground" data-content>{exception.fingerprint}</code>
            </li>
          {/each}
        </ul>
      </section>
    {/if}
    <form class="flex flex-col gap-4" novalidate onsubmit={(event) => void confirm(event)}>
      <Field.FieldGroup>
        <Field.Field>
          <Field.Label for={`${id}-hostname`}>{m.browser_certificate_hostname()}</Field.Label>
          <Input
            id={`${id}-hostname`}
            name="hostname"
            aria-describedby={`${id}-hostname-description`}
            autocomplete="off"
            data-browser-certificate-hostname=""
            maxlength={1024}
            required
            spellcheck={false}
          />
          <Field.Description id={`${id}-hostname-description`}>
            {m.browser_certificate_hostname_description()}
          </Field.Description>
        </Field.Field>
        <Field.Field>
          <Field.Label for={`${id}-fingerprint`}>{m.browser_certificate_fingerprint()}</Field.Label>
          <Input
            id={`${id}-fingerprint`}
            name="fingerprint"
            aria-describedby={`${id}-fingerprint-description`}
            autocomplete="off"
            data-browser-certificate-fingerprint=""
            maxlength={128}
            required
            spellcheck={false}
          />
          <Field.Description id={`${id}-fingerprint-description`}>
            {m.browser_certificate_fingerprint_description()}
          </Field.Description>
        </Field.Field>
      </Field.FieldGroup>
      <Dialog.Footer class="flex-wrap">
        <Dialog.Close>
          {#snippet child({ props })}
            <Button {...props} {disabled} type="button" variant="outline">{m.browser_close()}</Button>
          {/snippet}
        </Dialog.Close>
        {#if openedSessionId !== null}
          <Button data-browser-certificate-trust="" disabled={sessionTrustDisabled} type="submit" variant="outline">
            {m.browser_certificate_trust_session()}
          </Button>
        {/if}
        <Button data-browser-certificate-save="" {disabled} type="submit" value="saved">
          {m.browser_certificate_save()}
        </Button>
      </Dialog.Footer>
    </form>
  </Dialog.Content>
</Dialog.Root>

<span class="sr-only" role="status">{notice}</span>
