<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';

  import FieldHelp from '#lib/components/app/FieldHelp.svelte';
  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Dialog from '#lib/components/ui/dialog/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import { Input } from '#lib/components/ui/input/index.js';
  import * as RadioGroup from '#lib/components/ui/radio-group/index.js';
  import { Textarea } from '#lib/components/ui/textarea/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import type { SearchProfileForm } from '#lib/state/search-profile-form.svelte.js';
  import { profileFailureMessage } from '#lib/utils/chat-errors.js';
  import { SEARCH_KINDS, searchKindLabel } from '#lib/utils/provider-labels.js';

  // Adds or edits a search profile (ADR-0047): an embedding or rerank model on
  // one server. Each opening gets a new form. The credential field never shows
  // a stored credential, and only an embedding profile has prefixes.
  interface Props {
    /** The form of the latest opening, which stays while the dialog closes. */
    form: SearchProfileForm | null;
    open?: boolean;
  }

  let { form, open = $bindable(false) }: Props = $props();

  function credentialPlaceholder(current: SearchProfileForm): string {
    if (current.removesCredential) {
      return m.chat_providers_credential_removed_placeholder();
    }
    if (current.hasStoredCredential) {
      return m.chat_providers_credential_saved_placeholder();
    }
    return m.chat_providers_field_credential_placeholder();
  }

  /** Whether saving replaces the stored credential with the one typed into the field. */
  function replacesStoredCredential(current: SearchProfileForm): boolean {
    const replaces = current.credentialChange().action === 'replace';
    return current.hasStoredCredential && replaces;
  }

  /** `true` when the latest save failed with one of `codes`, for the field's `data-invalid`. */
  function failedWith(current: SearchProfileForm, ...codes: string[]): true | undefined {
    const code = current.failureCode;
    if (code === null) {
      return undefined;
    }
    return codes.includes(code) ? true : undefined;
  }

  async function save(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (form === null) {
      return;
    }
    const saved = await form.save();
    if (saved) {
      open = false;
    }
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-xl" showCloseButton={false}>
    {#if form !== null}
      <form class="flex flex-col gap-4" onsubmit={save}>
        <Dialog.Header>
          <Dialog.Title>
            {form.profile === null ? m.chat_search_add_title() : m.chat_search_edit_title()}
          </Dialog.Title>
          <Dialog.Description>{m.chat_providers_dialog_description()}</Dialog.Description>
        </Dialog.Header>
        <!-- The end padding holds the click area of a control at the end; with less, the fields scroll sideways. -->
        <div class="flex max-h-[60vh] flex-col gap-4 overflow-y-auto pe-3" data-search-profile-fields="">
          <Field.Group>
            <Field.Field data-invalid={failedWith(form, 'invalid_name', 'name_taken')}>
              <Field.Label for="search-profile-name">{m.chat_providers_field_name()}</Field.Label>
              <Input
                id="search-profile-name"
                placeholder={m.chat_search_field_name_placeholder()}
                bind:value={form.name}
              />
            </Field.Field>
            <!-- The legend also holds the help button, so the group takes its name from the label alone. -->
            <Field.Set aria-describedby="search-profile-kind-help" aria-labelledby="search-profile-kind-legend">
              <Field.Legend variant="label">
                <span class="flex items-center gap-1">
                  <span id="search-profile-kind-legend">{m.chat_search_field_kind()}</span>
                  <FieldHelp
                    id="search-profile-kind-help"
                    field={m.chat_search_field_kind()}
                    text={m.chat_search_field_kind_description()}
                  />
                </span>
              </Field.Legend>
              <RadioGroup.Root class="flex flex-wrap gap-x-6" orientation="horizontal" bind:value={form.kind}>
                {#each SEARCH_KINDS as option (option)}
                  <Field.Field class="w-auto" orientation="horizontal">
                    <RadioGroup.Item id="search-profile-kind-{option}" value={option} />
                    <Field.Label class="font-normal" for="search-profile-kind-{option}"
                      >{searchKindLabel(option)}</Field.Label
                    >
                  </Field.Field>
                {/each}
              </RadioGroup.Root>
            </Field.Set>
            <Field.Field data-invalid={failedWith(form, 'invalid_base_url', 'insecure_credential_transport')}>
              <div class="flex items-center gap-1">
                <Field.Label for="search-profile-base-url">{m.chat_providers_field_base_url()}</Field.Label>
                <FieldHelp
                  id="search-profile-base-url-help"
                  field={m.chat_providers_field_base_url()}
                  text={m.chat_search_field_base_url_description()}
                />
              </div>
              <Input
                id="search-profile-base-url"
                aria-describedby="search-profile-base-url-help"
                inputmode="url"
                placeholder={m.chat_search_field_base_url_placeholder()}
                bind:value={form.baseUrl}
              />
            </Field.Field>
            <Field.Field data-invalid={failedWith(form, 'invalid_model')}>
              <Field.Label for="search-profile-model">{m.chat_providers_field_model()}</Field.Label>
              <Input
                id="search-profile-model"
                placeholder={form.kind === 'rerank'
                  ? m.chat_search_field_rerank_model_placeholder()
                  : m.chat_search_field_embedding_model_placeholder()}
                bind:value={form.model}
              />
            </Field.Field>
            <Field.Field data-invalid={failedWith(form, 'invalid_credential')}>
              <div class="flex items-center gap-1">
                <Field.Label for="search-profile-credential">{m.chat_providers_field_credential_value()}</Field.Label>
                <FieldHelp
                  id="search-profile-credential-help"
                  field={m.chat_providers_field_credential_value()}
                  text={form.hasStoredCredential
                    ? m.chat_providers_field_credential_saved_description()
                    : m.chat_providers_field_credential_description()}
                />
              </div>
              <div class="flex gap-2">
                <Input
                  id="search-profile-credential"
                  aria-describedby="search-profile-credential-help"
                  autocomplete="off"
                  disabled={form.removesCredential}
                  placeholder={credentialPlaceholder(form)}
                  type="password"
                  bind:value={form.credential}
                />
                {#if form.hasStoredCredential && !form.removesCredential}
                  <Button type="button" variant="outline" onclick={() => form.removeStoredCredential()}>
                    {m.chat_providers_credential_remove()}
                  </Button>
                {/if}
              </div>
            </Field.Field>
            {#if form.removesCredential}
              <Alert.Root>
                <TriangleAlertIcon />
                <Alert.Description>{m.chat_providers_credential_remove_warning()}</Alert.Description>
                <Alert.Action>
                  <Button size="sm" type="button" variant="outline" onclick={() => form.keepStoredCredential()}>
                    {m.chat_providers_credential_keep()}
                  </Button>
                </Alert.Action>
              </Alert.Root>
            {:else if replacesStoredCredential(form)}
              <Alert.Root>
                <TriangleAlertIcon />
                <Alert.Description>{m.chat_providers_credential_replace_warning()}</Alert.Description>
              </Alert.Root>
            {/if}
            {#if form.takesPrefixes}
              <Field.Field data-invalid={failedWith(form, 'invalid_prefix')}>
                <div class="flex items-center gap-1">
                  <Field.Label for="search-profile-query-prefix">{m.chat_search_field_query_prefix()}</Field.Label>
                  <FieldHelp
                    id="search-profile-query-prefix-help"
                    field={m.chat_search_field_query_prefix()}
                    text={m.chat_search_field_query_prefix_description()}
                  />
                </div>
                <Textarea
                  id="search-profile-query-prefix"
                  class="min-h-16 font-mono text-xs"
                  aria-describedby="search-profile-query-prefix-help"
                  spellcheck={false}
                  bind:value={form.queryPrefix}
                />
              </Field.Field>
              <Field.Field data-invalid={failedWith(form, 'invalid_prefix')}>
                <div class="flex items-center gap-1">
                  <Field.Label for="search-profile-document-prefix">{m.chat_search_field_document_prefix()}</Field.Label
                  >
                  <FieldHelp
                    id="search-profile-document-prefix-help"
                    field={m.chat_search_field_document_prefix()}
                    text={m.chat_search_field_document_prefix_description()}
                  />
                </div>
                <Textarea
                  id="search-profile-document-prefix"
                  class="min-h-16 font-mono text-xs"
                  aria-describedby="search-profile-document-prefix-help"
                  spellcheck={false}
                  bind:value={form.documentPrefix}
                />
              </Field.Field>
            {/if}
          </Field.Group>
        </div>
        {#if form.failure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Title>{m.chat_providers_save_failed()}</Alert.Title>
            <Alert.Description>{profileFailureMessage(form.failure)}</Alert.Description>
          </Alert.Root>
        {/if}
        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={() => (open = false)}>{m.chat_cancel()}</Button>
          <Button disabled={form.saving} type="submit">{m.chat_save()}</Button>
        </Dialog.Footer>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>
