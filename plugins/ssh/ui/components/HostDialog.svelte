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

  import { m } from '../generated/messages.js';
  import {
    MAX_NAME_CHARS,
    MAX_NOTE_CHARS,
    MAX_USERNAME_CHARS,
    MIN_REDACTED_PASSWORD_BYTES,
  } from '../generated/plugin.js';
  import { sshActions } from '../state/ssh-actions.svelte.js';
  import { refusedField, sshFailureMessage } from '../utils/ssh-messages.js';

  // Adds or edits an SSH host (plugins/ssh/docs/README.md, settings): its
  // name, address, port, user name and password, and a note for agents.
  // Editing leaves the password empty, which keeps the stored one. The core
  // checks every field again; a field that it refuses is marked, and its
  // message stays in the dialog. A new host's key dialog opens next.
  let nameInput: HTMLInputElement | null = $state(null);

  const request = $derived(sshActions.edit);
  const isNew = $derived(request?.host === null);
  const refused = $derived(refusedField(request?.failure ?? null));

  /** The dialog opens in the Name field, which its help button precedes in the page. */
  function focusName(event: Event): void {
    if (nameInput !== null) {
      event.preventDefault();
      nameInput.focus();
    }
  }

  function submit(event: SubmitEvent): void {
    event.preventDefault();
    if (request === null) {
      return;
    }
    if (!request.saving) {
      void sshActions.confirmHost(request);
    }
  }
</script>

<Dialog.Root bind:open={sshActions.editOpen}>
  <Dialog.Content class="sm:max-w-lg" showCloseButton={false} onOpenAutoFocus={focusName}>
    {#if request !== null}
      <form class="flex flex-col gap-4" data-ssh-host-dialog="" onsubmit={submit}>
        <Dialog.Header>
          <Dialog.Title>
            {request.host === null ? m.ssh_add_title() : m.ssh_edit_title({ host: request.host.name })}
          </Dialog.Title>
          <Dialog.Description>{m.ssh_host_description()}</Dialog.Description>
        </Dialog.Header>
        <Field.Group>
          <Field.Field data-invalid={refused === 'name' || undefined}>
            <div class="flex items-center gap-1">
              <Field.Label for="ssh-host-name">{m.ssh_field_name()}</Field.Label>
              <FieldHelp id="ssh-host-name-help" field={m.ssh_field_name()} text={m.ssh_field_name_description()} />
            </div>
            <Input
              id="ssh-host-name"
              aria-describedby="ssh-host-name-help"
              aria-invalid={refused === 'name'}
              autocomplete="off"
              disabled={request.saving}
              maxlength={MAX_NAME_CHARS}
              required
              spellcheck="false"
              bind:ref={nameInput}
              bind:value={request.name}
            />
          </Field.Field>
          <div class="grid grid-cols-[minmax(0,1fr)_7rem] gap-3">
            <Field.Field data-invalid={refused === 'address' || undefined}>
              <div class="flex items-center gap-1">
                <Field.Label for="ssh-host-address">{m.ssh_field_address()}</Field.Label>
                <FieldHelp
                  id="ssh-host-address-help"
                  field={m.ssh_field_address()}
                  text={m.ssh_field_address_description()}
                />
              </div>
              <Input
                id="ssh-host-address"
                class="font-mono"
                aria-describedby="ssh-host-address-help"
                aria-invalid={refused === 'address'}
                autocomplete="off"
                dir="ltr"
                disabled={request.saving}
                required
                spellcheck="false"
                bind:value={request.address}
              />
            </Field.Field>
            <Field.Field data-invalid={refused === 'port' || undefined}>
              <Field.Label for="ssh-host-port">{m.ssh_field_port()}</Field.Label>
              <Input
                id="ssh-host-port"
                aria-invalid={refused === 'port'}
                disabled={request.saving}
                max="65535"
                min="1"
                required
                type="number"
                bind:value={request.port}
              />
            </Field.Field>
          </div>
          <Field.Field data-invalid={refused === 'username' || undefined}>
            <div class="flex items-center gap-1">
              <Field.Label for="ssh-host-username">{m.ssh_field_username()}</Field.Label>
              <FieldHelp
                id="ssh-host-username-help"
                field={m.ssh_field_username()}
                text={m.ssh_field_username_description()}
              />
            </div>
            <Input
              id="ssh-host-username"
              aria-describedby="ssh-host-username-help"
              aria-invalid={refused === 'username'}
              autocomplete="off"
              dir="ltr"
              disabled={request.saving}
              maxlength={MAX_USERNAME_CHARS}
              required
              spellcheck="false"
              bind:value={request.username}
            />
          </Field.Field>
          <Field.Field data-invalid={refused === 'password' || undefined}>
            <Field.Label for="ssh-host-password">{m.ssh_field_password()}</Field.Label>
            <Input
              id="ssh-host-password"
              aria-describedby="ssh-host-password-description"
              aria-invalid={refused === 'password'}
              autocomplete="off"
              disabled={request.saving}
              required={isNew}
              type="password"
              bind:value={request.password}
            />
            <Field.Description id="ssh-host-password-description">
              {#if !isNew}
                {m.ssh_field_password_keep()}
              {/if}
              {m.ssh_field_password_description({ count: MIN_REDACTED_PASSWORD_BYTES })}
            </Field.Description>
          </Field.Field>
          <Field.Field data-invalid={refused === 'note' || undefined}>
            <Field.Label for="ssh-host-note">{m.ssh_field_note()}</Field.Label>
            <Textarea
              id="ssh-host-note"
              class="max-h-40"
              aria-describedby="ssh-host-note-description"
              aria-invalid={refused === 'note'}
              disabled={request.saving}
              maxlength={MAX_NOTE_CHARS}
              bind:value={request.note}
            />
            <Field.Description id="ssh-host-note-description">{m.ssh_field_note_description()}</Field.Description>
          </Field.Field>
        </Field.Group>
        <div role="status">
          {#if request.saving}
            <p class="flex items-center gap-2 text-sm text-muted-foreground">
              <Spinner aria-hidden="true" />
              <span>{m.ssh_saving()}</span>
            </p>
          {/if}
        </div>
        {#if request.failure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Title>{m.ssh_save_failed_title()}</Alert.Title>
            <Alert.Description>{sshFailureMessage(request.failure)}</Alert.Description>
          </Alert.Root>
        {/if}
        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={() => (sshActions.editOpen = false)}>
            {m.ssh_cancel()}
          </Button>
          <Button data-ssh-host-submit="" disabled={request.saving} type="submit">
            {isNew ? m.ssh_add() : m.ssh_save()}
          </Button>
        </Dialog.Footer>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>
