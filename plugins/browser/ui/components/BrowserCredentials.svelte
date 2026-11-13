<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';

  import type { BrowserCredential, BrowserCredentialRef } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';

  interface Props {
    busy: boolean;
    credentials: BrowserCredentialRef[];
    failure: string | null;
    save: (action: BrowserCredential) => Promise<boolean>;
  }

  let { busy, credentials, failure, save }: Props = $props();
  const id = $props.id();
  let open = $state(false);

  async function add(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const form = event.currentTarget;
    if (!(form instanceof HTMLFormElement)) {
      return;
    }
    const data = new FormData(form);
    const origin = data.get('origin');
    const label = data.get('label');
    const value = data.get('value');
    const hasFields = typeof origin === 'string' && typeof label === 'string' && typeof value === 'string';
    if (!hasFields) {
      return;
    }
    const secret = form.elements.namedItem('value');
    if (secret instanceof HTMLInputElement) {
      secret.value = '';
    }
    if (await save({ kind: 'set', label, origin, value })) {
      form.reset();
    }
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Trigger>
    {#snippet child({ props })}
      <Button {...props} data-browser-credentials="" size="sm" variant="ghost">
        {m.browser_credentials()}
      </Button>
    {/snippet}
  </Dialog.Trigger>
  <Dialog.Content class="max-h-[90vh] overflow-y-auto">
    <Dialog.Header>
      <Dialog.Title>{m.browser_credentials()}</Dialog.Title>
      <Dialog.Description>{m.browser_credentials_description()}</Dialog.Description>
    </Dialog.Header>
    {#if failure !== null}
      <Alert.Root variant="destructive">
        <Alert.Title>{m.browser_error_title()}</Alert.Title>
        <Alert.Description>{failure}</Alert.Description>
      </Alert.Root>
    {/if}
    {#if credentials.length === 0}
      <p class="text-sm text-muted-foreground">{m.browser_credentials_empty()}</p>
    {:else}
      <ul class="flex flex-col gap-2">
        {#each credentials as credential (credential.reference)}
          <li class="flex flex-wrap items-center gap-2">
            <span class="me-auto text-sm wrap-anywhere" data-content dir="auto">
              {m.browser_credential_item({ label: credential.label, origin: credential.origin })}
            </span>
            <Button
              disabled={busy}
              size="sm"
              variant="outline"
              onclick={() => void save({ kind: 'remove', reference: credential.reference })}
            >
              {m.browser_credential_remove({ label: credential.label })}
            </Button>
          </li>
        {/each}
      </ul>
    {/if}
    <form class="flex flex-col gap-4" novalidate onsubmit={(event) => void add(event)}>
      <Field.FieldGroup>
        <Field.Field>
          <Field.Label for={`${id}-origin`}>{m.browser_credential_site()}</Field.Label>
          <Input id={`${id}-origin`} name="origin" required type="url" />
          <Field.Description>{m.browser_credential_site_description()}</Field.Description>
        </Field.Field>
        <Field.Field>
          <Field.Label for={`${id}-label`}>{m.browser_credential_label()}</Field.Label>
          <Input id={`${id}-label`} name="label" required />
        </Field.Field>
        <Field.Field>
          <Field.Label for={`${id}-secret`}>{m.browser_credential_value()}</Field.Label>
          <Input id={`${id}-secret`} name="value" autocomplete="off" required type="password" />
        </Field.Field>
      </Field.FieldGroup>
      <Dialog.Footer>
        <Dialog.Close>
          {#snippet child({ props })}
            <Button {...props} type="button" variant="outline">{m.browser_close()}</Button>
          {/snippet}
        </Dialog.Close>
        <Button disabled={busy} type="submit">{m.browser_credential_save()}</Button>
      </Dialog.Footer>
    </form>
  </Dialog.Content>
</Dialog.Root>
