<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import { onMount } from 'svelte';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import { Progress } from '#lib/components/ui/progress/index.js';
  import { Separator } from '#lib/components/ui/separator/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import { Switch } from '#lib/components/ui/switch/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { updates } from '#lib/state/updates.svelte.js';
  import {
    actionFailureTitle,
    checkTime,
    downloadedAmount,
    downloadedPercent,
    updateFailureText,
    updatesFailureMessage,
  } from '#lib/utils/updates-messages.js';

  // The settings section "Updates" (docs/features/updates.md): the version
  // that runs, what Mannie knows about a newer one, and the switch for
  // automatic checks. A build that does not update itself shows why, without
  // the check and the switch, which would do nothing there.
  const status = $derived(updates.status);
  const updatesOff = $derived(status?.state.kind === 'off');
  const checkAutomatically = $derived(updates.checkAutomatically);

  /** A running download: how much of it arrived, and the version that it brings. */
  const download = $derived.by(() => {
    const current = status?.state;
    if (current?.kind !== 'downloading') {
      return null;
    }
    return {
      amount: downloadedAmount(current.receivedBytes, current.totalBytes),
      percent: downloadedPercent(current.receivedBytes, current.totalBytes),
      version: current.version,
    };
  });

  /** A check that failed: when, why, and what the section says about it. */
  const failedCheck = $derived.by(() => {
    const current = status?.state;
    if (current?.kind !== 'failed') {
      return null;
    }
    return { checkedAt: current.checkedAt, code: current.code, text: updateFailureText(current.code) };
  });

  const offersReleasesPage = $derived.by(() => {
    const current = status?.state;
    if (current?.kind === 'off') {
      return current.reason === 'not_installed_from_package';
    }
    return failedCheck?.text.offersReleasesPage ?? false;
  });
  const hasActions = $derived(!updatesOff || offersReleasesPage);

  onMount(() => {
    updates.start();
    void updates.loadSettings();
  });
</script>

<div class="flex flex-col gap-6" data-updates-section="">
  {#if updates.failure !== null}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.updates_load_failed_title()}</Alert.Title>
      <Alert.Description>{updatesFailureMessage(updates.failure)}</Alert.Description>
      <Alert.Action>
        <Button size="sm" variant="outline" onclick={() => updates.retry()}>{m.updates_try_again()}</Button>
      </Alert.Action>
    </Alert.Root>
  {/if}

  {#if status !== null}
    {const current = $derived(status.state)}
    <div class="flex flex-col gap-3" data-updates-state={current.kind}>
      <p class="text-sm" data-updates-running-version="">
        {m.updates_running_version({ version: status.currentVersion })}
      </p>

      <div class="flex flex-col gap-1 text-sm text-muted-foreground" data-updates-status="" role="status">
        {#if current.kind === 'off'}
          <p>
            {current.reason === 'development_build' ? m.updates_off_development_build() : m.updates_off_not_installed()}
          </p>
        {:else if current.kind === 'idle'}
          {#if current.checkedAt === null}
            <p>{m.updates_not_checked()}</p>
          {:else}
            <p>{m.updates_up_to_date()}</p>
            <p>{m.updates_last_check({ time: checkTime(current.checkedAt) })}</p>
          {/if}
        {:else if current.kind === 'checking'}
          <p class="flex items-center gap-2">
            <Spinner aria-label={m.updates_checking()} />
            <span>{m.updates_checking()}</span>
          </p>
        {:else if current.kind === 'downloading'}
          <p>{m.updates_downloading({ version: current.version })}</p>
        {/if}
        {#if updates.readyVersion !== null}
          <p class="text-foreground" data-updates-ready="">{m.updates_ready({ version: updates.readyVersion })}</p>
          {#if status.needsSystemPassword}
            <p>{m.updates_needs_system_password()}</p>
          {/if}
        {/if}
        {#if updates.restarting}
          <p class="flex items-center gap-2">
            <Spinner aria-label={m.updates_restarting()} />
            <span>{m.updates_restarting()}</span>
          </p>
        {/if}
      </div>

      {#if download !== null}
        <!-- The amount stays outside the status region, which would announce every step of it. -->
        <div class="flex flex-col gap-1" data-updates-download="">
          {#if download.percent !== null}
            <Progress aria-label={m.updates_downloading({ version: download.version })} value={download.percent} />
          {/if}
          <p class="text-sm text-muted-foreground">{m.updates_downloaded({ amount: download.amount })}</p>
        </div>
      {/if}

      {#if failedCheck !== null}
        <Alert.Root data-updates-failed={failedCheck.code} variant="destructive">
          <CircleAlertIcon />
          <Alert.Title>{failedCheck.text.title}</Alert.Title>
          <Alert.Description>{failedCheck.text.description}</Alert.Description>
        </Alert.Root>
        <p class="text-sm text-muted-foreground">
          {m.updates_last_check({ time: checkTime(failedCheck.checkedAt) })}
        </p>
      {/if}

      {#if updates.actionFailure !== null}
        <Alert.Root data-updates-action-failed={updates.actionFailure.action} variant="destructive">
          <CircleAlertIcon />
          <Alert.Title>{actionFailureTitle(updates.actionFailure.action)}</Alert.Title>
          <Alert.Description>{updatesFailureMessage(updates.actionFailure.failure)}</Alert.Description>
        </Alert.Root>
      {/if}

      {#if hasActions}
        <div class="flex flex-wrap gap-2">
          {#if updates.readyVersion !== null}
            <Button data-updates-restart="" disabled={!updates.canRestart} onclick={() => void updates.restart()}>
              {m.updates_restart_and_update()}
            </Button>
            <Button data-updates-whats-new="" variant="outline" onclick={() => void updates.openReleasePage('ready')}>
              {m.updates_whats_new()}
            </Button>
          {/if}
          {#if !updatesOff}
            <Button
              data-updates-check=""
              disabled={!updates.canCheck}
              variant="outline"
              onclick={() => void updates.check()}
            >
              {m.updates_check_now()}
            </Button>
          {/if}
          {#if offersReleasesPage}
            <Button
              data-updates-releases-page=""
              variant="outline"
              onclick={() => void updates.openReleasePage('latest')}
            >
              {m.updates_open_releases_page()}
            </Button>
          {/if}
        </div>
      {/if}
    </div>

    {#if !updatesOff}
      <Separator />

      {#if updates.settingsFailure !== null}
        <Alert.Root variant="destructive">
          <CircleAlertIcon />
          <Alert.Title>{m.updates_settings_load_failed_title()}</Alert.Title>
          <Alert.Description>{updatesFailureMessage(updates.settingsFailure)}</Alert.Description>
          <Alert.Action>
            <Button size="sm" variant="outline" onclick={() => void updates.loadSettings()}>
              {m.updates_try_again()}
            </Button>
          </Alert.Action>
        </Alert.Root>
      {:else if checkAutomatically === null}
        <div class="flex flex-col gap-3" aria-busy="true">
          <Skeleton class="h-5 w-64" />
          <Skeleton class="h-5 w-full" />
        </div>
      {:else}
        <div class="flex flex-col gap-3">
          <Field.Field orientation="horizontal">
            <Field.Content>
              <Field.Label for="updates-check-automatically">{m.updates_automatic_label()}</Field.Label>
              <Field.Description id="updates-check-automatically-help">
                {m.updates_automatic_description()}
              </Field.Description>
            </Field.Content>
            <Switch
              id="updates-check-automatically"
              aria-describedby="updates-check-automatically-help"
              disabled={updates.saving}
              bind:checked={() => checkAutomatically, (checked: boolean) => void updates.setCheckAutomatically(checked)}
            />
          </Field.Field>
          {#if updates.saving}
            <p class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
              <Spinner aria-label={m.updates_settings_saving()} />
              <span>{m.updates_settings_saving()}</span>
            </p>
          {/if}
          {#if updates.saveFailure !== null}
            <Alert.Root variant="destructive">
              <CircleAlertIcon />
              <Alert.Title>{m.updates_settings_save_failed_title()}</Alert.Title>
              <Alert.Description>{updatesFailureMessage(updates.saveFailure)}</Alert.Description>
            </Alert.Root>
          {/if}
        </div>
      {/if}
    {/if}
  {:else if updates.failure === null}
    <div class="flex flex-col gap-3" aria-busy="true" role="status">
      <span class="sr-only">{m.updates_loading()}</span>
      <Skeleton class="h-5 w-48" />
      <Skeleton class="h-5 w-64" />
    </div>
  {/if}
</div>
