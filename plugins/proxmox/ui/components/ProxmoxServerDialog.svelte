<script lang="ts">
  import { FieldHelp } from '@mannie/plugin-sdk/components';
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import { Textarea } from '@mannie/plugin-sdk/components/textarea';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import TriangleAlertIcon from '@mannie/plugin-sdk/icons/triangle-alert';

  import { m } from '../generated/messages.js';
  import {
    DEFAULT_PORT,
    MAX_HOST_CHARS,
    MAX_NAME_CHARS,
    MAX_NOTE_CHARS,
    MAX_USERNAME_CHARS,
  } from '../generated/plugin.js';
  import { proxmoxActions } from '../state/proxmox-actions.svelte.js';
  import { proxmoxFailureMessage, refusedField } from '../utils/proxmox-messages.js';

  let nameInput: HTMLInputElement | null = $state(null);

  const plainNumbers = new Intl.NumberFormat(undefined, { useGrouping: false });

  const request = $derived(proxmoxActions.edit);
  const isNew = $derived(request?.server === null);
  const keepsStoredPassword = $derived(!isNew && request?.server?.passwordSet === true);
  const refused = $derived(refusedField(request?.failure ?? null));
  const endpointEdited = $derived(
    request !== null &&
      request.server !== null &&
      (request.host.trim() !== request.server.host || request.port !== request.server.port),
  );
  const passwordHelp = $derived(
    isNew
      ? m.proxmox_field_password_description()
      : `${m.proxmox_field_password_keep()} ${m.proxmox_field_password_description()}`,
  );
  const passwordPlaceholder = $derived(
    keepsStoredPassword ? m.proxmox_field_password_keep() : m.proxmox_field_password_placeholder(),
  );

  function focusName(event: Event): void {
    if (nameInput !== null) {
      event.preventDefault();
      nameInput.focus();
    }
  }

  function closeDialog(): void {
    proxmoxActions.editOpen = false;
  }

  function submit(event: SubmitEvent): void {
    event.preventDefault();
    if (request === null) {
      return;
    }

    if (request.saving) {
      return;
    }

    void proxmoxActions.confirmServer(request);
  }
</script>

<Dialog.Root bind:open={proxmoxActions.editOpen}>
  <Dialog.Content class="sm:max-w-lg" showCloseButton={false} onOpenAutoFocus={focusName}>
    {#if request !== null}
      <form class="flex flex-col gap-4" data-proxmox-server-dialog="" onsubmit={submit}>
        <Dialog.Header>
          <Dialog.Title>
            {request.server === null ? m.proxmox_add_title() : m.proxmox_edit_title({ server: request.server.name })}
          </Dialog.Title>
          <Dialog.Description>{m.proxmox_server_description()}</Dialog.Description>
        </Dialog.Header>

        <Field.Group>
          <Field.Field data-invalid={refused === 'name' || undefined}>
            <div class="flex items-center gap-1">
              <Field.Label for="proxmox-server-name">{m.proxmox_field_name()}</Field.Label>
              <FieldHelp
                id="proxmox-server-name-help"
                field={m.proxmox_field_name()}
                text={m.proxmox_field_name_description()}
              />
            </div>
            <Input
              id="proxmox-server-name"
              aria-describedby="proxmox-server-name-help"
              aria-invalid={refused === 'name'}
              autocomplete="off"
              disabled={request.saving}
              maxlength={MAX_NAME_CHARS}
              placeholder={m.proxmox_field_name_placeholder()}
              required
              spellcheck="false"
              bind:ref={nameInput}
              bind:value={request.name}
            />
          </Field.Field>

          <div class="grid grid-cols-[minmax(0,1fr)_7rem] gap-3">
            <Field.Field data-invalid={refused === 'host' || undefined}>
              <div class="flex items-center gap-1">
                <Field.Label for="proxmox-server-host">{m.proxmox_field_host()}</Field.Label>
                <FieldHelp
                  id="proxmox-server-host-help"
                  field={m.proxmox_field_host()}
                  text={m.proxmox_field_host_description()}
                />
              </div>
              <Input
                id="proxmox-server-host"
                class="font-mono"
                aria-describedby="proxmox-server-host-help"
                aria-invalid={refused === 'host'}
                autocomplete="off"
                dir="ltr"
                disabled={request.saving}
                maxlength={MAX_HOST_CHARS}
                placeholder={m.proxmox_field_host_placeholder()}
                required
                spellcheck="false"
                bind:value={request.host}
              />
            </Field.Field>

            <Field.Field data-invalid={refused === 'port' || undefined}>
              <Field.Label for="proxmox-server-port">{m.proxmox_field_port()}</Field.Label>
              <Input
                id="proxmox-server-port"
                aria-invalid={refused === 'port'}
                disabled={request.saving}
                max="65535"
                min="1"
                placeholder={plainNumbers.format(DEFAULT_PORT)}
                required
                type="number"
                bind:value={request.port}
              />
            </Field.Field>
          </div>

          {#if endpointEdited}
            <Alert.Root>
              <TriangleAlertIcon />
              <Alert.Description>{m.proxmox_edit_endpoint_warning()}</Alert.Description>
            </Alert.Root>
          {/if}

          <Field.Field data-invalid={refused === 'username' || undefined}>
            <div class="flex items-center gap-1">
              <Field.Label for="proxmox-server-username">{m.proxmox_field_username()}</Field.Label>
              <FieldHelp
                id="proxmox-server-username-help"
                field={m.proxmox_field_username()}
                text={m.proxmox_field_username_description()}
              />
            </div>
            <Input
              id="proxmox-server-username"
              aria-describedby="proxmox-server-username-help"
              aria-invalid={refused === 'username'}
              autocomplete="off"
              dir="ltr"
              disabled={request.saving}
              maxlength={MAX_USERNAME_CHARS}
              placeholder={m.proxmox_field_username_placeholder()}
              required
              spellcheck="false"
              bind:value={request.username}
            />
          </Field.Field>

          <Field.Field data-invalid={refused === 'password' || undefined}>
            <div class="flex items-center gap-1">
              <Field.Label for="proxmox-server-password">{m.proxmox_field_password()}</Field.Label>
              <FieldHelp id="proxmox-server-password-help" field={m.proxmox_field_password()} text={passwordHelp} />
            </div>
            <Input
              id="proxmox-server-password"
              aria-describedby="proxmox-server-password-help"
              aria-invalid={refused === 'password'}
              autocomplete="off"
              disabled={request.saving}
              placeholder={passwordPlaceholder}
              required={isNew}
              type="password"
              bind:value={request.password}
            />
          </Field.Field>

          <Field.Field data-invalid={refused === 'note' || undefined}>
            <div class="flex items-center gap-1">
              <Field.Label for="proxmox-server-note">{m.proxmox_field_note()}</Field.Label>
              <FieldHelp
                id="proxmox-server-note-help"
                field={m.proxmox_field_note()}
                text={m.proxmox_field_note_description()}
              />
            </div>
            <Textarea
              id="proxmox-server-note"
              class="max-h-40"
              aria-describedby="proxmox-server-note-help"
              aria-invalid={refused === 'note'}
              disabled={request.saving}
              maxlength={MAX_NOTE_CHARS}
              placeholder={m.proxmox_field_note_placeholder()}
              bind:value={request.note}
            />
          </Field.Field>
        </Field.Group>

        <div role="status">
          {#if request.saving}
            <p class="flex items-center gap-2 text-sm text-muted-foreground">
              <Spinner aria-hidden="true" />
              <span>{m.proxmox_saving()}</span>
            </p>
          {/if}
        </div>

        {#if request.failure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Title>{m.proxmox_save_failed_title()}</Alert.Title>
            <Alert.Description>{proxmoxFailureMessage(request.failure)}</Alert.Description>
          </Alert.Root>
        {/if}

        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={closeDialog}>
            {m.proxmox_cancel()}
          </Button>
          <Button data-proxmox-server-submit="" disabled={request.saving} type="submit">
            {isNew ? m.proxmox_add() : m.proxmox_save()}
          </Button>
        </Dialog.Footer>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>
