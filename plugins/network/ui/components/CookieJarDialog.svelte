<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { Checkbox } from '@mannie/plugin-sdk/components/checkbox';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import { Switch } from '@mannie/plugin-sdk/components/switch';
  import { formatTime } from '@mannie/plugin-sdk/format';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import PencilIcon from '@mannie/plugin-sdk/icons/pencil';
  import TrashIcon from '@mannie/plugin-sdk/icons/trash';

  import type { CookieView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { networkDialogs } from '../state/network-dialogs.svelte.js';
  import { networkLibrary } from '../state/network.svelte.js';
  import type { NetworkFailure } from '../types/network.js';
  import { networkFailureMessage } from '../utils/network-messages.js';

  let editingId = $state<string | null>(null);
  let editingExpiresAt = $state<number | null>(null);
  let name = $state('');
  let value = $state('');
  let domain = $state('');
  let path = $state('/');
  let hostOnly = $state(false);
  let secure = $state(true);
  let httpOnly = $state(false);
  let enabled = $state(true);
  let saving = $state(false);
  let failure = $state<NetworkFailure | null>(null);

  const submitLabel = $derived.by(() => {
    if (saving) {
      return m.network_saving();
    }
    return editingId === null ? m.network_add() : m.network_save();
  });

  function resetForm(): void {
    editingId = null;
    editingExpiresAt = null;
    name = '';
    value = '';
    domain = '';
    path = '/';
    hostOnly = false;
    secure = true;
    httpOnly = false;
    enabled = true;
    failure = null;
  }

  function editCookie(cookie: CookieView): void {
    editingId = cookie.id;
    editingExpiresAt = cookie.expiresAt;
    name = cookie.name;
    value = cookie.value;
    domain = cookie.domain;
    path = cookie.path;
    hostOnly = cookie.hostOnly ?? false;
    secure = cookie.secure;
    httpOnly = cookie.httpOnly;
    enabled = cookie.enabled;
    failure = null;
  }

  function handleOpenChange(nextOpen: boolean): void {
    if (!nextOpen && !saving) {
      networkDialogs.closeCookieJar();
    }
  }

  async function submitCookie(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const missingName = name.trim() === '';
    const missingDomain = domain.trim() === '';

    if (missingName || missingDomain) {
      return;
    }
    saving = true;
    failure = null;
    const outcome = await networkLibrary.saveCookie({
      cookieId: editingId,
      domain,
      enabled,
      expiresAt: editingId === null ? null : editingExpiresAt,
      hostOnly,
      httpOnly,
      name,
      path,
      secure,
      value,
    });

    saving = false;
    if (outcome.ok) {
      resetForm();
    } else {
      failure = outcome.failure;
    }
  }

  async function toggleCookieEnabled(cookie: CookieView, nextEnabled: boolean): Promise<void> {
    saving = true;
    failure = null;
    const outcome = await networkLibrary.saveCookie({
      cookieId: cookie.id,
      domain: cookie.domain,
      enabled: nextEnabled,
      expiresAt: cookie.expiresAt,
      hostOnly: cookie.hostOnly,
      httpOnly: cookie.httpOnly,
      name: cookie.name,
      path: cookie.path,
      secure: cookie.secure,
      value: cookie.value,
    });

    saving = false;
    if (!outcome.ok) {
      failure = outcome.failure;
    }
  }

  async function removeCookie(cookieId: string): Promise<void> {
    saving = true;
    failure = null;
    const outcome = await networkLibrary.deleteCookie(cookieId);

    saving = false;
    if (!outcome.ok) {
      failure = outcome.failure;
    } else if (editingId === cookieId) {
      resetForm();
    }
  }

  $effect(() => {
    if (networkDialogs.cookieJarOpen) {
      resetForm();
      saving = false;
    }
  });
</script>

<Dialog.Root open={networkDialogs.cookieJarOpen} onOpenChange={handleOpenChange}>
  <Dialog.Content class="flex max-h-[calc(100vh-2rem)] flex-col overflow-hidden sm:max-w-3xl" showCloseButton={false}>
    <div class="flex min-h-0 flex-1 flex-col gap-4">
      <Dialog.Header class="shrink-0">
        <div class="flex items-center justify-between gap-2">
          <Dialog.Title>{m.network_cookies_title()}</Dialog.Title>
          {#if networkLibrary.cookies.length > 0}
            <Button
              disabled={saving}
              size="xs"
              type="button"
              variant="outline"
              onclick={() => networkDialogs.confirmDelete({ kind: 'clear_cookies' })}
            >
              {m.network_clear_all()}
            </Button>
          {/if}
        </div>
        <Dialog.Description>{m.network_cookies_description()}</Dialog.Description>
      </Dialog.Header>

      <div class="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto pe-1">
        {#if failure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Description>{networkFailureMessage(failure)}</Alert.Description>
          </Alert.Root>
        {/if}

        {#if networkLibrary.cookies.length === 0}
          <p class="text-xs text-muted-foreground">{m.network_cookies_empty()}</p>
        {:else}
          <div class="max-h-56 overflow-auto rounded-md border border-border">
            <table class="w-full border-collapse text-xs">
              <thead>
                <tr class="border-b border-border bg-muted/40 text-muted-foreground">
                  <th class="w-8 p-2 text-center font-medium">
                    <span class="sr-only">{m.network_enabled()}</span>
                  </th>
                  <th class="p-2 text-start font-medium">{m.network_cookies_name_label()}</th>
                  <th class="p-2 text-start font-medium">{m.network_cookies_value_label()}</th>
                  <th class="p-2 text-start font-medium">{m.network_cookies_domain_label()}</th>
                  <th class="p-2 text-start font-medium">{m.network_cookies_expires_label()}</th>
                  <th class="w-20 p-2 text-end font-medium">
                    <span class="sr-only">{m.network_edit()}</span>
                  </th>
                </tr>
              </thead>

              <tbody>
                {#each networkLibrary.cookies as cookie (cookie.id)}
                  <tr class="border-b border-border last:border-b-0">
                    <td class="p-1.5 text-center">
                      <Checkbox
                        aria-label={m.network_enabled()}
                        checked={cookie.enabled}
                        disabled={saving}
                        onCheckedChange={(checked: boolean) => void toggleCookieEnabled(cookie, checked)}
                      />
                    </td>
                    <td class="p-1.5 font-mono font-medium" data-content="" dir="ltr">
                      {cookie.name}
                      {#if cookie.secure}
                        <Badge class="ms-1 h-4 rounded-sm px-1 text-[9px]" variant="secondary">
                          {m.network_secret()}
                        </Badge>
                      {/if}
                    </td>
                    <td class="max-w-44 truncate p-1.5 font-mono" data-content="" dir="ltr" title={cookie.value}>
                      {cookie.value}
                    </td>
                    <td class="p-1.5 font-mono" data-content="" dir="ltr">
                      {cookie.domain}{cookie.path}
                      {#if cookie.hostOnly}
                        <Badge class="ms-1 h-4 rounded-sm px-1 text-[9px]" variant="outline">
                          {m.network_cookies_host_only_badge()}
                        </Badge>
                      {/if}
                    </td>
                    <td class="p-1.5 text-muted-foreground" data-content="">
                      {cookie.expiresAt === null ? m.network_cookies_session() : formatTime(cookie.expiresAt)}
                    </td>
                    <td class="p-1.5 text-end">
                      <div class="inline-flex items-center gap-0.5">
                        <Button
                          aria-label={m.network_edit()}
                          disabled={saving}
                          size="icon-xs"
                          type="button"
                          variant="ghost"
                          onclick={() => editCookie(cookie)}
                        >
                          <PencilIcon class="size-3.5" />
                        </Button>

                        <Button
                          aria-label={m.network_delete()}
                          disabled={saving}
                          size="icon-xs"
                          type="button"
                          variant="ghost"
                          onclick={() => void removeCookie(cookie.id)}
                        >
                          <TrashIcon class="size-3.5" />
                        </Button>
                      </div>
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}

        <form
          class="flex flex-col gap-3 rounded-lg border border-border bg-muted/20 p-3"
          onsubmit={(event: SubmitEvent) => void submitCookie(event)}
        >
          <h3 class="text-xs font-semibold">{m.network_cookies_add_heading()}</h3>

          <div class="grid gap-3 sm:grid-cols-2">
            <Field.Field>
              <Field.Label>{m.network_cookies_name_label()}</Field.Label>
              <Input
                class="font-mono text-xs"
                disabled={saving}
                placeholder={m.network_cookies_name_placeholder()}
                required
                bind:value={name}
              />
            </Field.Field>

            <Field.Field>
              <Field.Label>{m.network_cookies_value_label()}</Field.Label>
              <Input
                class="font-mono text-xs"
                disabled={saving}
                placeholder={m.network_cookies_value_placeholder()}
                bind:value
              />
            </Field.Field>

            <Field.Field>
              <Field.Label>{m.network_cookies_domain_label()}</Field.Label>
              <Input
                class="font-mono text-xs"
                disabled={saving}
                placeholder={m.network_cookies_domain_placeholder()}
                required
                bind:value={domain}
              />
            </Field.Field>

            <Field.Field>
              <Field.Label>{m.network_cookies_path_label()}</Field.Label>
              <Input
                class="font-mono text-xs"
                disabled={saving}
                placeholder={m.network_cookies_path_placeholder()}
                bind:value={path}
              />
            </Field.Field>
          </div>

          <div class="flex flex-wrap items-center gap-6">
            <label class="flex items-center gap-2 text-xs">
              <Switch aria-label={m.network_cookies_host_only_label()} disabled={saving} bind:checked={hostOnly} />
              <span>{m.network_cookies_host_only_label()}</span>
            </label>
            <label class="flex items-center gap-2 text-xs">
              <Switch aria-label={m.network_cookies_secure_label()} disabled={saving} bind:checked={secure} />
              <span>{m.network_cookies_secure_label()}</span>
            </label>
            <label class="flex items-center gap-2 text-xs">
              <Switch aria-label={m.network_cookies_http_only_label()} disabled={saving} bind:checked={httpOnly} />
              <span>{m.network_cookies_http_only_label()}</span>
            </label>
          </div>

          <div class="flex justify-end gap-2">
            {#if editingId !== null}
              <Button disabled={saving} size="sm" type="button" variant="ghost" onclick={resetForm}>
                {m.network_cancel()}
              </Button>
            {/if}
            <Button disabled={saving || name.trim() === '' || domain.trim() === ''} size="sm" type="submit">
              {submitLabel}
            </Button>
          </div>
        </form>
      </div>

      <Dialog.Footer class="shrink-0">
        <Button disabled={saving} type="button" variant="outline" onclick={() => networkDialogs.closeCookieJar()}>
          {m.network_close()}
        </Button>
      </Dialog.Footer>
    </div>
  </Dialog.Content>
</Dialog.Root>
