<script lang="ts">
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Item from '@mannie/plugin-sdk/components/item';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';

  import type { HostStatus, HostView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { sshActions } from '../state/ssh-actions.svelte.js';
  import { hostStatusLabel, sshFailureMessage } from '../utils/ssh-messages.js';

  // One host in the SSH settings (plugins/ssh/docs/README.md, settings):
  // its name, its status, where its commands run, the owner's note and the
  // fingerprint of its trusted key, with the owner's actions. The name,
  // endpoint and note are content that the owner wrote, and the endpoint
  // and fingerprint read left to right in every language.
  let { host }: { host: HostView } = $props();

  const test = $derived(sshActions.tests[host.id] ?? null);
  const isReady = $derived(host.status === 'ready');
  const isTesting = $derived(test?.status === 'testing');
  const testPassed = $derived(test?.status === 'passed');
  const testFailure = $derived(test?.failure ?? null);

  /** How the badge marks a status: a changed key stands out. */
  function statusVariant(status: HostStatus): 'destructive' | 'outline' | 'secondary' {
    switch (status) {
      case 'key_changed': {
        return 'destructive';
      }
      case 'ready': {
        return 'secondary';
      }
      case 'key_not_checked':
      case 'password_missing': {
        return 'outline';
      }
    }
  }
</script>

<Item.Root data-ssh-host={host.name} role="listitem" variant="outline">
  <Item.Content class="min-w-0 gap-2">
    <div class="flex min-w-0 flex-wrap items-center gap-2">
      <Item.Title class="min-w-0 wrap-anywhere" data-content>{host.name}</Item.Title>
      <Badge data-ssh-status={host.status} variant={statusVariant(host.status)}>{hostStatusLabel(host.status)}</Badge>
    </div>
    <p class="font-mono text-sm wrap-anywhere text-muted-foreground" data-content dir="ltr">{host.endpoint}</p>
    {#if host.note !== ''}
      <p class="text-sm wrap-anywhere whitespace-pre-wrap" data-content dir="auto">{host.note}</p>
    {/if}
    {#if host.key !== null}
      <p class="font-mono text-xs wrap-anywhere text-muted-foreground" data-content dir="ltr">
        {host.key.fingerprint}
      </p>
    {/if}
    <div class="text-sm" role="status">
      {#if isTesting}
        <p class="flex items-center gap-2 text-muted-foreground">
          <Spinner aria-hidden="true" />
          <span>{m.ssh_testing({ host: host.name })}</span>
        </p>
      {:else if testPassed}
        <p data-ssh-test="passed">{m.ssh_test_passed({ host: host.name })}</p>
      {:else if testFailure !== null}
        <p class="text-destructive" data-ssh-test="failed">
          {m.ssh_test_failed({ host: host.name, reason: sshFailureMessage(testFailure) })}
        </p>
      {/if}
    </div>
  </Item.Content>
  <Item.Actions class="flex-wrap self-start">
    <Button data-ssh-check-key="" size="sm" variant="outline" onclick={() => sshActions.startCheckKey(host)}>
      {m.ssh_check_key()}
    </Button>
    <Button
      data-ssh-test-login=""
      disabled={!isReady || isTesting}
      size="sm"
      variant="outline"
      onclick={() => void sshActions.test(host)}
    >
      {m.ssh_test_login()}
    </Button>
    <Button data-ssh-edit="" size="sm" variant="outline" onclick={() => sshActions.startEdit(host)}>
      {m.ssh_edit()}
    </Button>
    <Button data-ssh-remove="" size="sm" variant="outline" onclick={() => sshActions.startRemove(host)}>
      {m.ssh_remove()}
    </Button>
  </Item.Actions>
</Item.Root>
