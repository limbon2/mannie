<script lang="ts">
  import { FieldHelp } from '@mannie/plugin-sdk/components';
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import * as RadioGroup from '@mannie/plugin-sdk/components/radio-group';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import { Switch } from '@mannie/plugin-sdk/components/switch';
  import { Textarea } from '@mannie/plugin-sdk/components/textarea';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import TriangleAlertIcon from '@mannie/plugin-sdk/icons/triangle-alert';

  import { m } from '../generated/messages.js';
  import {
    DEFAULT_TIMEOUT_SECONDS,
    MAX_CLIENT_ID_CHARS,
    MAX_FOLDER_CHARS,
    MAX_NAME_CHARS,
    MAX_PREFIX_CHARS,
    MAX_PROGRAM_CHARS,
    MAX_TIMEOUT_SECONDS,
    MAX_URL_BYTES,
    MIN_TIMEOUT_SECONDS,
  } from '../generated/plugin.js';
  import { mcpActions } from '../state/mcp-actions.svelte.js';
  import { mcpFailureMessage, refusedField } from '../utils/mcp-messages.js';

  import ValueRows from './ValueRows.svelte';

  // Adds or edits an MCP server (plugins/mcp/docs/README.md, registering a
  // server): a program on this computer with its arguments, folder and
  // variables, or a remote URL with its headers and its sign-in, and the
  // server's switches. A server keeps its kind. The core checks every field
  // again; a field that it refuses is marked, and its message stays in the
  // dialog. Secret values never come back: an empty field keeps a stored one.
  // Only the fields scroll, so the title and the buttons stay in view.
  let nameInput: HTMLInputElement | null = $state(null);

  const plainNumbers = new Intl.NumberFormat(undefined, { useGrouping: false });

  const request = $derived(mcpActions.edit);
  const isNew = $derived(request?.server === null);
  const isStdio = $derived(request?.transport === 'stdio');
  const refused = $derived(refusedField(request?.failure ?? null));
  const keepsClientSecret = $derived(request?.server?.clientSecretSet === true);
  const clientSecretHelp = $derived(
    keepsClientSecret
      ? `${m.mcp_field_client_secret_description()} ${m.mcp_field_client_secret_keep()}`
      : m.mcp_field_client_secret_description(),
  );
  const clientSecretPlaceholder = $derived(
    keepsClientSecret ? m.mcp_secret_stored() : m.mcp_field_client_secret_placeholder(),
  );

  /** The dialog opens in the Name field, which its help button precedes in the page. */
  function focusName(event: Event): void {
    if (nameInput !== null) {
      event.preventDefault();
      nameInput.focus();
    }
  }

  function chooseTransport(value: string): void {
    const isClosed = request === null;
    if (isClosed || !isNew) {
      return;
    }
    const isTransport = value === 'stdio' || value === 'http';
    if (isTransport) {
      request.transport = value;
    }
  }

  function submit(event: SubmitEvent): void {
    event.preventDefault();
    if (request === null) {
      return;
    }
    if (!request.saving) {
      void mcpActions.confirmServer(request);
    }
  }
</script>

<Dialog.Root bind:open={mcpActions.editOpen}>
  <Dialog.Content class="flex max-h-[90vh] flex-col sm:max-w-2xl" showCloseButton={false} onOpenAutoFocus={focusName}>
    {#if request !== null}
      <form class="flex min-h-0 flex-col gap-4" data-mcp-server-dialog="" onsubmit={submit}>
        <Dialog.Header>
          <Dialog.Title>
            {request.server === null ? m.mcp_add_title() : m.mcp_edit_title({ server: request.server.name })}
          </Dialog.Title>
          <Dialog.Description>{m.mcp_server_dialog_description()}</Dialog.Description>
        </Dialog.Header>
        <div class="relative -mx-4 flex min-h-0 flex-col gap-4 overflow-y-auto px-4 py-1">
          <Field.Set>
            <Field.Legend variant="label">{m.mcp_field_transport()}</Field.Legend>
            <RadioGroup.Root
              class="flex flex-wrap gap-x-6"
              disabled={!isNew || request.saving}
              orientation="horizontal"
              value={request.transport}
              onValueChange={chooseTransport}
            >
              <Field.Field class="w-auto" orientation="horizontal">
                <RadioGroup.Item id="mcp-transport-stdio" value="stdio" />
                <Field.Label class="font-normal" for="mcp-transport-stdio">{m.mcp_transport_stdio()}</Field.Label>
              </Field.Field>
              <Field.Field class="w-auto" orientation="horizontal">
                <RadioGroup.Item id="mcp-transport-http" value="http" />
                <Field.Label class="font-normal" for="mcp-transport-http">{m.mcp_transport_http()}</Field.Label>
              </Field.Field>
            </RadioGroup.Root>
          </Field.Set>
          <Field.Group>
            <div class="grid grid-cols-[minmax(0,1fr)_10rem] gap-3">
              <Field.Field data-invalid={refused === 'name' || undefined}>
                <div class="flex items-center gap-1">
                  <Field.Label for="mcp-server-name">{m.mcp_field_name()}</Field.Label>
                  <FieldHelp
                    id="mcp-server-name-help"
                    field={m.mcp_field_name()}
                    text={m.mcp_field_name_description()}
                  />
                </div>
                <Input
                  id="mcp-server-name"
                  aria-describedby="mcp-server-name-help"
                  aria-invalid={refused === 'name'}
                  autocomplete="off"
                  disabled={request.saving}
                  maxlength={MAX_NAME_CHARS}
                  placeholder={m.mcp_field_name_placeholder()}
                  required
                  spellcheck="false"
                  bind:ref={nameInput}
                  bind:value={request.name}
                />
              </Field.Field>
              <Field.Field data-invalid={refused === 'prefix' || undefined}>
                <div class="flex items-center gap-1">
                  <Field.Label for="mcp-server-prefix">{m.mcp_field_prefix()}</Field.Label>
                  <FieldHelp
                    id="mcp-server-prefix-help"
                    field={m.mcp_field_prefix()}
                    text={m.mcp_field_prefix_description()}
                  />
                </div>
                <Input
                  id="mcp-server-prefix"
                  class="font-mono"
                  aria-describedby="mcp-server-prefix-help"
                  aria-invalid={refused === 'prefix'}
                  autocomplete="off"
                  dir="ltr"
                  disabled={request.saving}
                  maxlength={MAX_PREFIX_CHARS}
                  placeholder={m.mcp_field_prefix_placeholder()}
                  spellcheck="false"
                  bind:value={request.prefix}
                />
              </Field.Field>
            </div>
            {#if isStdio}
              <Field.Field data-invalid={refused === 'program' || undefined}>
                <div class="flex items-center gap-1">
                  <Field.Label for="mcp-server-program">{m.mcp_field_program()}</Field.Label>
                  <FieldHelp
                    id="mcp-server-program-help"
                    field={m.mcp_field_program()}
                    text={m.mcp_field_program_description()}
                  />
                </div>
                <div class="flex gap-2">
                  <Input
                    id="mcp-server-program"
                    class="font-mono"
                    aria-describedby="mcp-server-program-help"
                    aria-invalid={refused === 'program'}
                    autocomplete="off"
                    dir="ltr"
                    disabled={request.saving}
                    maxlength={MAX_PROGRAM_CHARS}
                    placeholder={m.mcp_field_program_placeholder()}
                    required
                    spellcheck="false"
                    bind:value={request.program}
                  />
                  <Button
                    data-mcp-locate=""
                    disabled={request.saving || request.locating || request.program.trim() === ''}
                    type="button"
                    variant="outline"
                    onclick={() => void mcpActions.locate(request)}
                  >
                    {m.mcp_locate()}
                  </Button>
                </div>
                <div class="text-sm" role="status">
                  {#if request.locating}
                    <p class="flex items-center gap-2 text-muted-foreground">
                      <Spinner aria-hidden="true" />
                      <span>{m.mcp_locating()}</span>
                    </p>
                  {:else if request.located !== null}
                    <p class="wrap-anywhere" data-mcp-located="">
                      {m.mcp_located({ path: request.located.path })}
                    </p>
                    {#if request.located.inProject}
                      <p class="flex items-center gap-2 text-destructive">
                        <TriangleAlertIcon class="size-4 shrink-0" aria-hidden="true" />
                        <span>{m.mcp_located_in_project()}</span>
                      </p>
                    {/if}
                  {:else if request.locateFailure !== null}
                    <p class="text-destructive">
                      {m.mcp_locate_failed({ reason: mcpFailureMessage(request.locateFailure) })}
                    </p>
                  {/if}
                </div>
              </Field.Field>
              <Field.Field data-invalid={refused === 'arguments' || undefined}>
                <div class="flex items-center gap-1">
                  <Field.Label for="mcp-server-arguments">{m.mcp_field_arguments()}</Field.Label>
                  <FieldHelp
                    id="mcp-server-arguments-help"
                    field={m.mcp_field_arguments()}
                    text={m.mcp_field_arguments_description()}
                  />
                </div>
                <Textarea
                  id="mcp-server-arguments"
                  class="max-h-40 font-mono"
                  aria-describedby="mcp-server-arguments-help"
                  aria-invalid={refused === 'arguments'}
                  dir="ltr"
                  disabled={request.saving}
                  placeholder={m.mcp_field_arguments_placeholder()}
                  spellcheck="false"
                  bind:value={request.argumentsText}
                />
              </Field.Field>
              <Field.Field data-invalid={refused === 'folder' || undefined}>
                <div class="flex items-center gap-1">
                  <Field.Label for="mcp-server-folder">{m.mcp_field_folder()}</Field.Label>
                  <FieldHelp
                    id="mcp-server-folder-help"
                    field={m.mcp_field_folder()}
                    text={m.mcp_field_folder_description()}
                  />
                </div>
                <Input
                  id="mcp-server-folder"
                  class="font-mono placeholder:font-sans"
                  aria-describedby="mcp-server-folder-help"
                  aria-invalid={refused === 'folder'}
                  autocomplete="off"
                  dir="ltr"
                  disabled={request.saving}
                  maxlength={MAX_FOLDER_CHARS}
                  placeholder={m.mcp_field_folder_placeholder()}
                  spellcheck="false"
                  bind:value={request.folder}
                />
              </Field.Field>
              <ValueRows
                id="mcp-server-variables"
                disabled={request.saving}
                help={m.mcp_field_variables_description()}
                invalid={refused === 'plain'}
                legend={m.mcp_field_variables()}
                namePlaceholder={m.mcp_variable_name_placeholder()}
                rows={request.plain}
                secret={false}
                valuePlaceholder={m.mcp_variable_value_placeholder()}
                onAdd={() => mcpActions.addPlain(request)}
                onRemove={(index: number) => mcpActions.removePlain(request, index)}
              />
              <ValueRows
                id="mcp-server-secret-variables"
                disabled={request.saving}
                help={m.mcp_field_secret_variables_description()}
                invalid={refused === 'secrets'}
                legend={m.mcp_field_secret_variables()}
                namePlaceholder={m.mcp_secret_variable_name_placeholder()}
                rows={request.secrets}
                secret
                valuePlaceholder={m.mcp_secret_variable_value_placeholder()}
                onAdd={() => mcpActions.addSecret(request)}
                onRemove={(index: number) => mcpActions.removeSecret(request, index)}
              />
            {:else}
              <Field.Field data-invalid={refused === 'url' || undefined}>
                <div class="flex items-center gap-1">
                  <Field.Label for="mcp-server-url">{m.mcp_field_url()}</Field.Label>
                  <FieldHelp id="mcp-server-url-help" field={m.mcp_field_url()} text={m.mcp_field_url_description()} />
                </div>
                <Input
                  id="mcp-server-url"
                  class="font-mono"
                  aria-describedby="mcp-server-url-help"
                  aria-invalid={refused === 'url'}
                  autocomplete="off"
                  dir="ltr"
                  disabled={request.saving}
                  maxlength={MAX_URL_BYTES}
                  placeholder={m.mcp_field_url_placeholder()}
                  required
                  spellcheck="false"
                  type="url"
                  bind:value={request.url}
                />
              </Field.Field>
              <ValueRows
                id="mcp-server-headers"
                disabled={request.saving}
                help={m.mcp_field_headers_description()}
                invalid={refused === 'plain'}
                legend={m.mcp_field_headers()}
                namePlaceholder={m.mcp_header_name_placeholder()}
                rows={request.plain}
                secret={false}
                valuePlaceholder={m.mcp_header_value_placeholder()}
                onAdd={() => mcpActions.addPlain(request)}
                onRemove={(index: number) => mcpActions.removePlain(request, index)}
              />
              <ValueRows
                id="mcp-server-secret-headers"
                disabled={request.saving}
                help={m.mcp_field_secret_headers_description()}
                invalid={refused === 'secrets'}
                legend={m.mcp_field_secret_headers()}
                namePlaceholder={m.mcp_secret_header_name_placeholder()}
                rows={request.secrets}
                secret
                valuePlaceholder={m.mcp_secret_header_value_placeholder()}
                onAdd={() => mcpActions.addSecret(request)}
                onRemove={(index: number) => mcpActions.removeSecret(request, index)}
              />
              <Field.Field data-invalid={refused === 'oauth' || undefined} orientation="horizontal">
                <div class="flex flex-auto items-center gap-1">
                  <Field.Label for="mcp-server-oauth">{m.mcp_field_oauth()}</Field.Label>
                  <FieldHelp
                    id="mcp-server-oauth-help"
                    field={m.mcp_field_oauth()}
                    text={m.mcp_field_oauth_description()}
                  />
                </div>
                <Switch
                  id="mcp-server-oauth"
                  aria-describedby="mcp-server-oauth-help"
                  disabled={request.saving}
                  bind:checked={request.oauth}
                />
              </Field.Field>
              {#if request.oauth}
                <Field.Field data-invalid={refused === 'clientId' || undefined}>
                  <div class="flex items-center gap-1">
                    <Field.Label for="mcp-server-client-id">{m.mcp_field_client_id()}</Field.Label>
                    <FieldHelp
                      id="mcp-server-client-id-help"
                      field={m.mcp_field_client_id()}
                      text={m.mcp_field_client_id_description()}
                    />
                  </div>
                  <Input
                    id="mcp-server-client-id"
                    class="font-mono placeholder:font-sans"
                    aria-describedby="mcp-server-client-id-help"
                    aria-invalid={refused === 'clientId'}
                    autocomplete="off"
                    dir="ltr"
                    disabled={request.saving}
                    maxlength={MAX_CLIENT_ID_CHARS}
                    placeholder={m.mcp_field_client_id_placeholder()}
                    spellcheck="false"
                    bind:value={request.clientId}
                  />
                </Field.Field>
                {#if request.clientId.trim() !== ''}
                  <Field.Field data-invalid={refused === 'clientSecret' || undefined}>
                    <div class="flex items-center gap-1">
                      <Field.Label for="mcp-server-client-secret">{m.mcp_field_client_secret()}</Field.Label>
                      <FieldHelp
                        id="mcp-server-client-secret-help"
                        field={m.mcp_field_client_secret()}
                        text={clientSecretHelp}
                      />
                    </div>
                    <Input
                      id="mcp-server-client-secret"
                      aria-describedby="mcp-server-client-secret-help"
                      aria-invalid={refused === 'clientSecret'}
                      autocomplete="off"
                      disabled={request.saving || request.forgetClientSecret}
                      placeholder={clientSecretPlaceholder}
                      type="password"
                      bind:value={request.clientSecret}
                    />
                  </Field.Field>
                  {#if keepsClientSecret}
                    <Field.Field orientation="horizontal">
                      <Switch
                        id="mcp-server-forget-client-secret"
                        disabled={request.saving}
                        bind:checked={request.forgetClientSecret}
                      />
                      <Field.Label class="font-normal" for="mcp-server-forget-client-secret">
                        {m.mcp_field_forget_client_secret()}
                      </Field.Label>
                    </Field.Field>
                  {/if}
                {/if}
              {/if}
            {/if}
            {#each request.moved as name (name)}
              <p class="text-sm text-muted-foreground">{m.mcp_secret_moved({ name })}</p>
            {/each}
            <Field.Field orientation="horizontal">
              <div class="flex flex-auto items-center gap-1">
                <Field.Label for="mcp-server-images">{m.mcp_field_images()}</Field.Label>
                <FieldHelp
                  id="mcp-server-images-help"
                  field={m.mcp_field_images()}
                  text={m.mcp_field_images_description()}
                />
              </div>
              <Switch
                id="mcp-server-images"
                aria-describedby="mcp-server-images-help"
                disabled={request.saving}
                bind:checked={request.images}
              />
            </Field.Field>
            <Field.Field orientation="horizontal">
              <div class="flex flex-auto items-center gap-1">
                <Field.Label for="mcp-server-instructions">{m.mcp_field_instructions()}</Field.Label>
                <FieldHelp
                  id="mcp-server-instructions-help"
                  field={m.mcp_field_instructions()}
                  text={m.mcp_field_instructions_description()}
                />
              </div>
              <Switch
                id="mcp-server-instructions"
                aria-describedby="mcp-server-instructions-help"
                disabled={request.saving}
                bind:checked={request.instructions}
              />
            </Field.Field>
            <Field.Field class="max-w-56" data-invalid={refused === 'timeoutSeconds' || undefined}>
              <div class="flex items-center gap-1">
                <Field.Label for="mcp-server-timeout">{m.mcp_field_timeout()}</Field.Label>
                <FieldHelp
                  id="mcp-server-timeout-help"
                  field={m.mcp_field_timeout()}
                  text={m.mcp_field_timeout_description({ max: MAX_TIMEOUT_SECONDS, min: MIN_TIMEOUT_SECONDS })}
                />
              </div>
              <Input
                id="mcp-server-timeout"
                aria-describedby="mcp-server-timeout-help"
                aria-invalid={refused === 'timeoutSeconds'}
                disabled={request.saving}
                max={MAX_TIMEOUT_SECONDS}
                min={MIN_TIMEOUT_SECONDS}
                placeholder={plainNumbers.format(DEFAULT_TIMEOUT_SECONDS)}
                required
                type="number"
                bind:value={request.timeoutSeconds}
              />
            </Field.Field>
          </Field.Group>
        </div>
        <div role="status">
          {#if request.saving}
            <p class="flex items-center gap-2 text-sm text-muted-foreground">
              <Spinner aria-hidden="true" />
              <span>{m.mcp_saving()}</span>
            </p>
          {/if}
        </div>
        {#if request.failure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Title>{m.mcp_save_failed_title()}</Alert.Title>
            <Alert.Description>{mcpFailureMessage(request.failure)}</Alert.Description>
          </Alert.Root>
        {/if}
        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={() => (mcpActions.editOpen = false)}>
            {m.mcp_cancel()}
          </Button>
          <Button data-mcp-server-submit="" disabled={request.saving} type="submit">
            {isNew ? m.mcp_add() : m.mcp_save()}
          </Button>
        </Dialog.Footer>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>
