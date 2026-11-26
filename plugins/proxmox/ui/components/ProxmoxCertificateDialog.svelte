<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import ShieldCheckIcon from '@mannie/plugin-sdk/icons/shield-check';
  import TriangleAlertIcon from '@mannie/plugin-sdk/icons/triangle-alert';

  import { m } from '../generated/messages.js';
  import { proxmoxActions } from '../state/proxmox-actions.svelte.js';
  import { proxmoxFailureMessage } from '../utils/proxmox-messages.js';

  const request = $derived(proxmoxActions.cert);
  const info = $derived(request?.info ?? null);
  const pinnedFingerprint = $derived(request?.server.pinnedCertFingerprint ?? null);
  const isBusy = $derived(request !== null && (request.inspecting || request.pinning || request.clearing));
  const isMismatch = $derived(info !== null && info.matchesPin === false);
  const matchesPin = $derived(info !== null && info.matchesPin === true);
  const canPin = $derived(info !== null && info.validNow && !matchesPin && !isBusy);
  const canClearPin = $derived(pinnedFingerprint !== null && !isBusy);
  const canContinueToVms = $derived(
    request !== null &&
      request.continueToVms &&
      info !== null &&
      info.validNow &&
      (matchesPin || (pinnedFingerprint === null && info.osTrusted)) &&
      !isBusy,
  );

  function closeDialog(): void {
    proxmoxActions.certOpen = false;
  }

  function inspectAgain(): void {
    if (request !== null) {
      void proxmoxActions.inspectCertificate(request);
    }
  }

  function pinCertificate(): void {
    if (request !== null) {
      void proxmoxActions.confirmPinCertificate(request);
    }
  }

  function clearPin(): void {
    if (request !== null) {
      void proxmoxActions.clearCertificatePin(request);
    }
  }

  function continueToVms(): void {
    if (request !== null) {
      proxmoxActions.certOpen = false;
      proxmoxActions.startChooseVms(request.server);
    }
  }
</script>

<Dialog.Root bind:open={proxmoxActions.certOpen}>
  <Dialog.Content class="sm:max-w-lg" showCloseButton={false}>
    {#if request !== null}
      <div class="flex flex-col gap-4" data-proxmox-cert-dialog="">
        <Dialog.Header>
          <Dialog.Title>{m.proxmox_cert_title({ server: request.server.name })}</Dialog.Title>
          <Dialog.Description>{m.proxmox_cert_description()}</Dialog.Description>
        </Dialog.Header>

        {#if info !== null}
          <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1.5 text-sm">
            {#if pinnedFingerprint !== null}
              <dt class="text-muted-foreground">{m.proxmox_cert_pinned_fingerprint()}</dt>
              <dd class="font-mono text-xs wrap-anywhere" data-content data-proxmox-pinned-fingerprint="" dir="ltr">
                {pinnedFingerprint}
              </dd>
            {/if}

            <dt class="text-muted-foreground">{m.proxmox_cert_fingerprint()}</dt>
            <dd class="font-mono text-xs wrap-anywhere" data-content data-proxmox-cert-fingerprint="" dir="ltr">
              {info.fingerprint}
            </dd>

            <dt class="text-muted-foreground">{m.proxmox_cert_subject()}</dt>
            <dd class="wrap-anywhere" data-content dir="ltr">{info.subject}</dd>

            <dt class="text-muted-foreground">{m.proxmox_cert_issuer()}</dt>
            <dd class="wrap-anywhere" data-content dir="ltr">{info.issuer}</dd>

            {#if info.sans.length > 0}
              <dt class="text-muted-foreground">{m.proxmox_cert_sans()}</dt>
              <dd class="font-mono text-xs wrap-anywhere" data-content dir="ltr">{info.sans.join(', ')}</dd>
            {/if}

            <dt class="text-muted-foreground">{m.proxmox_cert_validity()}</dt>
            <dd class="font-mono text-xs wrap-anywhere" data-content dir="ltr">
              {info.notBefore} / {info.notAfter}
            </dd>
          </dl>

          {#if !info.validNow}
            <Alert.Root variant="destructive">
              <TriangleAlertIcon />
              <Alert.Description>{m.proxmox_cert_expired_warning()}</Alert.Description>
            </Alert.Root>
          {:else if isMismatch}
            <Alert.Root variant="destructive">
              <TriangleAlertIcon />
              <Alert.Description>{m.proxmox_cert_mismatch_warning()}</Alert.Description>
            </Alert.Root>
          {:else if matchesPin}
            <Alert.Root>
              <ShieldCheckIcon />
              <Alert.Description>{m.proxmox_cert_matches_pin()}</Alert.Description>
            </Alert.Root>
          {:else if info.osTrusted}
            <Alert.Root>
              <ShieldCheckIcon />
              <Alert.Description>{m.proxmox_cert_os_trusted()}</Alert.Description>
            </Alert.Root>
          {:else}
            <Alert.Root>
              <TriangleAlertIcon />
              <Alert.Description>{m.proxmox_cert_not_os_trusted()}</Alert.Description>
            </Alert.Root>
          {/if}
        {/if}

        <div class="text-sm" role="status">
          {#if request.inspecting}
            <p class="flex items-center gap-2 text-muted-foreground">
              <Spinner aria-hidden="true" />
              <span>{m.proxmox_cert_inspecting()}</span>
            </p>
          {:else if request.pinning}
            <p class="flex items-center gap-2 text-muted-foreground">
              <Spinner aria-hidden="true" />
              <span>{m.proxmox_cert_pinning()}</span>
            </p>
          {:else if request.clearing}
            <p class="flex items-center gap-2 text-muted-foreground">
              <Spinner aria-hidden="true" />
              <span>{m.proxmox_cert_clearing()}</span>
            </p>
          {/if}
        </div>

        {#if request.failure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Title>{m.proxmox_cert_failed_title()}</Alert.Title>
            <Alert.Description>{proxmoxFailureMessage(request.failure)}</Alert.Description>
          </Alert.Root>
        {/if}

        <Dialog.Footer class="flex-wrap gap-2">
          <Button type="button" variant="outline" onclick={closeDialog}>
            {m.proxmox_close()}
          </Button>
          <Button
            data-proxmox-cert-inspect-again=""
            disabled={isBusy}
            type="button"
            variant="outline"
            onclick={inspectAgain}
          >
            {m.proxmox_cert_inspect_again()}
          </Button>
          {#if canClearPin}
            <Button data-proxmox-cert-clear="" type="button" variant="outline" onclick={clearPin}>
              {m.proxmox_cert_clear_pin()}
            </Button>
          {/if}

          {#if canPin}
            <Button data-proxmox-cert-pin="" type="button" onclick={pinCertificate}>
              {m.proxmox_cert_pin_action()}
            </Button>
          {:else if canContinueToVms}
            <Button data-proxmox-cert-continue="" type="button" onclick={continueToVms}>
              {m.proxmox_cert_continue_vms()}
            </Button>
          {/if}
        </Dialog.Footer>
      </div>
    {/if}
  </Dialog.Content>
</Dialog.Root>
