<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import TriangleAlertIcon from '@mannie/plugin-sdk/icons/triangle-alert';
  import { call } from '@mannie/plugin-sdk/ipc';

  import { type BrowserRejectedCertificate, commands } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import {
    browserCertificateReasonMessage,
    browserFailureMessage,
    formatCertificateFingerprint,
  } from '../utils/browser-messages.js';

  interface Props {
    canRetry: boolean;
    certificate: BrowserRejectedCertificate | null;
    onTrusted: (url: string) => void;
    open?: boolean;
  }

  let { canRetry, certificate, onTrusted, open = $bindable(false) }: Props = $props();

  let busy = $state(false);
  let saveFailure = $state<string | null>(null);

  const hasCertificate = $derived(certificate !== null);
  const isRevoked = $derived(certificate?.reason === 'revoked');
  const canSave = $derived(hasCertificate && !isRevoked && !busy);
  const formattedFingerprint = $derived(
    certificate === null ? '' : formatCertificateFingerprint(certificate.fingerprint),
  );
  const formattedValidity = $derived(
    certificate === null ? '' : formatValidityWindow(certificate.notBefore, certificate.notAfter),
  );
  const reasonLabel = $derived(certificate === null ? '' : browserCertificateReasonMessage(certificate.reason));
  const trustLabel = $derived(
    canRetryCertificate(canRetry, certificate)
      ? m.browser_certificate_inspect_save_retry()
      : m.browser_certificate_save(),
  );

  function canRetryCertificate(retryEnabled: boolean, current: BrowserRejectedCertificate | null): boolean {
    if (!retryEnabled) {
      return false;
    }

    if (current === null) {
      return false;
    }

    return current.url !== '';
  }

  function formatTimestamp(iso: string): string {
    const parsed = new Date(iso);
    if (Number.isNaN(parsed.getTime())) {
      return iso;
    }

    return new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(parsed);
  }

  function formatValidityWindow(notBefore: string, notAfter: string): string {
    return `${formatTimestamp(notBefore)} / ${formatTimestamp(notAfter)}`;
  }

  function handleOpenChange(nextOpen: boolean): void {
    if (nextOpen) {
      saveFailure = null;
    }
  }

  function closeDialog(): void {
    open = false;
  }

  function startSaveTrust(): void {
    void saveTrust();
  }

  async function saveTrust(): Promise<void> {
    if (certificate === null) {
      return;
    }

    if (isRevoked || busy) {
      return;
    }

    const { fingerprint, hostname, url } = certificate;

    busy = true;
    saveFailure = null;

    try {
      const outcome = await call(() => commands.browserSaveCertificate({ fingerprint, hostname }));
      if (outcome.ok) {
        open = false;
        onTrusted(url);
      } else {
        saveFailure = browserFailureMessage(outcome.failure);
      }
    } finally {
      busy = false;
    }
  }
</script>

<Dialog.Root bind:open onOpenChange={handleOpenChange}>
  <Dialog.Content class="sm:max-w-lg" showCloseButton={false}>
    {#if certificate !== null}
      <div class="flex flex-col gap-4" data-browser-certificate-inspect-dialog="">
        <Dialog.Header>
          <Dialog.Title>{m.browser_certificate_inspect_title({ hostname: certificate.hostname })}</Dialog.Title>
          <Dialog.Description>
            {m.browser_certificate_inspect_description({ hostname: certificate.hostname })}
          </Dialog.Description>
        </Dialog.Header>

        <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1.5 text-sm">
          <dt class="text-muted-foreground">{m.browser_certificate_inspect_reason()}</dt>
          <dd class="wrap-anywhere" data-browser-inspect-reason="">{reasonLabel}</dd>

          <dt class="text-muted-foreground">{m.browser_certificate_hostname()}</dt>
          <dd class="font-mono text-xs wrap-anywhere" data-browser-inspect-hostname="" data-content dir="ltr">
            {certificate.hostname}
          </dd>

          <dt class="text-muted-foreground">{m.browser_certificate_fingerprint()}</dt>
          <dd class="font-mono text-xs wrap-anywhere" data-browser-inspect-fingerprint="" data-content dir="ltr">
            {formattedFingerprint}
          </dd>

          <dt class="text-muted-foreground">{m.browser_certificate_inspect_subject()}</dt>
          <dd class="wrap-anywhere" data-browser-inspect-subject="" data-content dir="ltr">{certificate.subject}</dd>

          <dt class="text-muted-foreground">{m.browser_certificate_inspect_issuer()}</dt>
          <dd class="wrap-anywhere" data-browser-inspect-issuer="" data-content dir="ltr">
            {certificate.issuer}
            <span class="block text-xs text-muted-foreground">
              {certificate.selfSigned
                ? m.browser_certificate_inspect_self_signed()
                : m.browser_certificate_inspect_ca_signed()}
            </span>
          </dd>

          {#if certificate.sans.length > 0}
            <dt class="text-muted-foreground">{m.browser_certificate_inspect_sans()}</dt>
            <dd class="font-mono text-xs wrap-anywhere" data-content dir="ltr">{certificate.sans.join(', ')}</dd>
          {/if}

          <dt class="text-muted-foreground">{m.browser_certificate_inspect_validity()}</dt>
          <dd class="text-xs wrap-anywhere" data-content dir="ltr">{formattedValidity}</dd>
        </dl>

        {#if isRevoked}
          <Alert.Root data-browser-inspect-revoked-warning="" variant="destructive">
            <CircleAlertIcon />
            <Alert.Description>{m.browser_certificate_inspect_revoked_warning()}</Alert.Description>
          </Alert.Root>
        {:else if certificate.expired}
          <Alert.Root data-browser-inspect-expired-warning="" variant="destructive">
            <TriangleAlertIcon />
            <Alert.Description>{m.browser_certificate_inspect_expired_warning()}</Alert.Description>
          </Alert.Root>
        {/if}

        {#if busy}
          <p class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
            <Spinner aria-hidden="true" aria-label={m.browser_certificate_inspect_saving()} />
            <span>{m.browser_certificate_inspect_saving()}</span>
          </p>
        {/if}

        {#if saveFailure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Title>{m.browser_certificate_change_failed()}</Alert.Title>
            <Alert.Description>{saveFailure}</Alert.Description>
          </Alert.Root>
        {/if}

        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={closeDialog}>{m.browser_cancel()}</Button>
          <Button data-browser-certificate-inspect-trust="" disabled={!canSave} type="button" onclick={startSaveTrust}>
            {trustLabel}
          </Button>
        </Dialog.Footer>
      </div>
    {/if}
  </Dialog.Content>
</Dialog.Root>
