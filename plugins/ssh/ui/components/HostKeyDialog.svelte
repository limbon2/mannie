<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import TriangleAlertIcon from '@mannie/plugin-sdk/icons/triangle-alert';

  import { m } from '../generated/messages.js';
  import { sshActions } from '../state/ssh-actions.svelte.js';
  import { sshFailureMessage } from '../utils/ssh-messages.js';

  // Checks a host's key (plugins/ssh/docs/README.md, host keys): the dialog
  // reads the key that the host shows, without logging in, and shows its
  // fingerprint for the owner to compare with the host's own. "Trust this
  // key" pins it and then tests the login. A key that differs from the
  // trusted one shows both fingerprints and a warning; nothing goes to the
  // host until the owner trusts the new key.
  const request = $derived(sshActions.key);
  const scan = $derived(request?.scan ?? null);
  const pinned = $derived(request?.host.key ?? null);
  const isChanged = $derived(scan?.comparison === 'changed');
  const isSame = $derived(scan?.comparison === 'same');
  const isBusy = $derived(request !== null && (request.scanning || request.trusting || request.testing));
  const canTrust = $derived(scan !== null && !isSame && !isBusy && request?.tested === false);
  const failureTitle = $derived(titleOf(request?.failedStep ?? null));

  function titleOf(step: 'scan' | 'test' | 'trust' | null): string {
    switch (step) {
      case 'scan': {
        return m.ssh_scan_failed_title();
      }
      case 'test': {
        return m.ssh_test_failed_title();
      }
      case 'trust':
      case null: {
        return m.ssh_trust_failed_title();
      }
    }
  }
</script>

<Dialog.Root bind:open={sshActions.keyOpen}>
  <Dialog.Content class="sm:max-w-lg" showCloseButton={false}>
    {#if request !== null}
      <div class="flex flex-col gap-4" data-ssh-key-dialog="">
        <Dialog.Header>
          <Dialog.Title>{m.ssh_key_title({ host: request.host.name })}</Dialog.Title>
          <Dialog.Description>{m.ssh_key_description()}</Dialog.Description>
        </Dialog.Header>
        {#if scan !== null}
          <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-sm">
            {#if isChanged}
              {#if pinned !== null}
                <dt class="text-muted-foreground">{m.ssh_key_trusted()}</dt>
                <dd class="font-mono wrap-anywhere" data-content data-ssh-pinned-fingerprint="" dir="ltr">
                  {pinned.fingerprint}
                </dd>
              {/if}
            {/if}
            <dt class="text-muted-foreground">{m.ssh_key_algorithm()}</dt>
            <dd class="font-mono wrap-anywhere" data-content dir="ltr">{scan.key.algorithm}</dd>
            <dt class="text-muted-foreground">{isChanged ? m.ssh_key_shown() : m.ssh_key_fingerprint()}</dt>
            <dd class="font-mono wrap-anywhere" data-content data-ssh-fingerprint="" dir="ltr">
              {scan.key.fingerprint}
            </dd>
          </dl>
          {#if isChanged}
            <Alert.Root variant="destructive">
              <TriangleAlertIcon />
              <Alert.Description>{m.ssh_key_changed_warning()}</Alert.Description>
            </Alert.Root>
          {:else if isSame}
            <p class="text-sm">{m.ssh_key_same()}</p>
          {/if}
        {/if}
        <div class="text-sm" role="status">
          {#if request.scanning}
            <p class="flex items-center gap-2 text-muted-foreground">
              <Spinner aria-hidden="true" />
              <span>{m.ssh_key_scanning()}</span>
            </p>
          {:else if request.trusting}
            <p class="flex items-center gap-2 text-muted-foreground">
              <Spinner aria-hidden="true" />
              <span>{m.ssh_trusting()}</span>
            </p>
          {:else if request.testing}
            <p class="flex items-center gap-2 text-muted-foreground">
              <Spinner aria-hidden="true" />
              <span>{m.ssh_testing({ host: request.host.name })}</span>
            </p>
          {:else if request.tested}
            <p data-ssh-key-tested="">{m.ssh_test_passed({ host: request.host.name })}</p>
          {/if}
        </div>
        {#if request.failure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Title>{failureTitle}</Alert.Title>
            <Alert.Description>{sshFailureMessage(request.failure)}</Alert.Description>
          </Alert.Root>
        {/if}
        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={() => (sshActions.keyOpen = false)}>
            {m.ssh_close()}
          </Button>
          {#if request.failedStep === 'scan'}
            <Button data-ssh-scan-again="" disabled={isBusy} onclick={() => void sshActions.scan(request)}>
              {m.ssh_scan_again()}
            </Button>
          {:else if canTrust}
            <Button data-ssh-trust="" onclick={() => void sshActions.confirmTrust(request)}>
              {m.ssh_trust()}
            </Button>
          {/if}
        </Dialog.Footer>
      </div>
    {/if}
  </Dialog.Content>
</Dialog.Root>
