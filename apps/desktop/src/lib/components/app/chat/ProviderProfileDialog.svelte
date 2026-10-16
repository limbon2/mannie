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
  import * as Select from '#lib/components/ui/select/index.js';
  import { Switch } from '#lib/components/ui/switch/index.js';
  import type { ApiInterface } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import type { ProviderProfileForm } from '#lib/state/provider-profile-form.svelte.js';
  import { formatCount, profileFailureMessage } from '#lib/utils/chat-errors.js';
  import { effortLabel, interfaceLabel, REASONING_EFFORTS } from '#lib/utils/provider-labels.js';

  // Adds or edits a provider profile (docs/features/chat/conversation-view.md,
  // settings sections). Each opening gets a new form. The credential field
  // never shows a stored credential: its placeholder says that one is saved,
  // and the dialog warns before saving replaces or removes it.
  interface Props {
    /** The form of the latest opening, which stays while the dialog closes. */
    form: ProviderProfileForm | null;
    open?: boolean;
  }

  let { form, open = $bindable(false) }: Props = $props();

  const INTERFACES: readonly ApiInterface[] = ['responses', 'chat_completions'];

  // Number fields take digits without separators, so their examples show none.
  const plainNumbers = new Intl.NumberFormat(undefined, { useGrouping: false });

  const efforts = $derived(REASONING_EFFORTS.map((value) => ({ label: effortLabel(value), value })));

  function credentialPlaceholder(current: ProviderProfileForm): string {
    if (current.removesCredential) {
      return m.chat_providers_credential_removed_placeholder();
    }
    if (current.hasStoredCredential) {
      return m.chat_providers_credential_saved_placeholder();
    }
    return m.chat_providers_field_credential_placeholder();
  }

  /** Whether saving replaces the stored credential with the one typed into the field. */
  function replacesStoredCredential(current: ProviderProfileForm): boolean {
    const replaces = current.credentialChange().action === 'replace';
    return current.hasStoredCredential && replaces;
  }

  /** `true` when the latest save failed with one of `codes`, for the field's `data-invalid`. */
  function failedWith(current: ProviderProfileForm, ...codes: string[]): true | undefined {
    const code = current.failureCode;
    if (code === null) {
      return undefined;
    }
    return codes.includes(code) ? true : undefined;
  }

  /** `true` when a limit is empty or the core refused the limits. */
  function limitsInvalid(current: ProviderProfileForm): true | undefined {
    if (current.limitsMissing) {
      return true;
    }
    return failedWith(current, 'invalid_limits');
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

  function touchLimits(): void {
    if (form !== null) {
      form.limitsTouched = true;
    }
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-xl" showCloseButton={false}>
    {#if form !== null}
      <form class="flex flex-col gap-4" onsubmit={save}>
        <Dialog.Header>
          <Dialog.Title>
            {form.profile === null ? m.chat_providers_add_title() : m.chat_providers_edit_title()}
          </Dialog.Title>
          <Dialog.Description>{m.chat_providers_dialog_description()}</Dialog.Description>
        </Dialog.Header>
        <!-- The end padding holds the click area of a control at the end, which reaches 12 pixels past the switch; with less, the fields scroll sideways. -->
        <div class="flex max-h-[60vh] flex-col gap-4 overflow-y-auto pe-3" data-profile-fields="">
          <Field.Group>
            <Field.Field data-invalid={failedWith(form, 'invalid_name', 'name_taken')}>
              <Field.Label for="profile-name">{m.chat_providers_field_name()}</Field.Label>
              <Input id="profile-name" placeholder={m.chat_providers_field_name_placeholder()} bind:value={form.name} />
            </Field.Field>
            <!-- The legend also holds the help button, so the group takes its name from the label alone. -->
            <Field.Set aria-describedby="profile-interface-help" aria-labelledby="profile-interface-legend">
              <Field.Legend variant="label">
                <span class="flex items-center gap-1">
                  <span id="profile-interface-legend">{m.chat_providers_field_interface()}</span>
                  <FieldHelp
                    id="profile-interface-help"
                    field={m.chat_providers_field_interface()}
                    text={m.chat_providers_field_interface_description()}
                  />
                </span>
              </Field.Legend>
              <RadioGroup.Root class="flex flex-wrap gap-x-6" orientation="horizontal" bind:value={form.api}>
                {#each INTERFACES as option (option)}
                  <Field.Field class="w-auto" orientation="horizontal">
                    <RadioGroup.Item id="profile-interface-{option}" value={option} />
                    <Field.Label class="font-normal" for="profile-interface-{option}"
                      >{interfaceLabel(option)}</Field.Label
                    >
                  </Field.Field>
                {/each}
              </RadioGroup.Root>
            </Field.Set>
            <Field.Field data-invalid={failedWith(form, 'invalid_base_url', 'insecure_credential_transport')}>
              <div class="flex items-center gap-1">
                <Field.Label for="profile-base-url">{m.chat_providers_field_base_url()}</Field.Label>
                <FieldHelp
                  id="profile-base-url-help"
                  field={m.chat_providers_field_base_url()}
                  text={m.chat_providers_field_base_url_description()}
                />
              </div>
              <Input
                id="profile-base-url"
                aria-describedby="profile-base-url-help"
                inputmode="url"
                placeholder={m.chat_providers_field_base_url_placeholder()}
                bind:value={form.baseUrl}
              />
            </Field.Field>
            <Field.Field data-invalid={failedWith(form, 'invalid_model')}>
              <Field.Label for="profile-model">{m.chat_providers_field_model()}</Field.Label>
              <Input
                id="profile-model"
                placeholder={m.chat_providers_field_model_placeholder()}
                bind:value={form.model}
                onblur={() => void form.applyModelDefaults()}
              />
              {#if form.defaultsApplied}
                <Field.Description>{m.chat_providers_defaults_applied()}</Field.Description>
              {/if}
            </Field.Field>
            <Field.Field data-invalid={failedWith(form, 'invalid_credential')}>
              <div class="flex items-center gap-1">
                <Field.Label for="profile-credential">{m.chat_providers_field_credential_value()}</Field.Label>
                <FieldHelp
                  id="profile-credential-help"
                  field={m.chat_providers_field_credential_value()}
                  text={form.hasStoredCredential
                    ? m.chat_providers_field_credential_saved_description()
                    : m.chat_providers_field_credential_description()}
                />
              </div>
              <div class="flex gap-2">
                <Input
                  id="profile-credential"
                  aria-describedby="profile-credential-help"
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
            <div class="grid gap-4 sm:grid-cols-2">
              <Field.Field data-invalid={limitsInvalid(form)}>
                <Field.Label for="profile-context-window">{m.chat_providers_field_context_window()}</Field.Label>
                <Input
                  id="profile-context-window"
                  min="1024"
                  placeholder={plainNumbers.format(128_000)}
                  type="number"
                  bind:value={form.contextWindow}
                  oninput={touchLimits}
                />
              </Field.Field>
              <Field.Field data-invalid={limitsInvalid(form)}>
                <Field.Label for="profile-output-limit">{m.chat_providers_field_output_limit()}</Field.Label>
                <Input
                  id="profile-output-limit"
                  min="16"
                  placeholder={plainNumbers.format(16_384)}
                  type="number"
                  bind:value={form.outputLimit}
                  oninput={touchLimits}
                />
              </Field.Field>
            </div>
            <Field.Field>
              <div class="flex items-center gap-1">
                <Field.Label for="profile-effort">{m.chat_providers_field_effort()}</Field.Label>
                <FieldHelp
                  id="profile-effort-help"
                  field={m.chat_providers_field_effort()}
                  text={m.chat_providers_field_effort_description()}
                />
              </div>
              <Select.Root items={efforts} type="single" bind:value={form.effort}>
                <Select.Trigger id="profile-effort" class="w-full" aria-describedby="profile-effort-help">
                  {effortLabel(form.effort)}
                </Select.Trigger>
                <Select.Content>
                  {#each efforts as option (option.value)}
                    <Select.Item label={option.label} value={option.value}>{option.label}</Select.Item>
                  {/each}
                </Select.Content>
              </Select.Root>
            </Field.Field>
            <Field.Field orientation="horizontal">
              <div class="flex flex-auto items-center gap-1">
                <Field.Label for="profile-use-tools">{m.chat_providers_field_use_tools()}</Field.Label>
                <FieldHelp
                  id="profile-use-tools-help"
                  field={m.chat_providers_field_use_tools()}
                  text={m.chat_providers_field_use_tools_description()}
                />
              </div>
              <Switch id="profile-use-tools" aria-describedby="profile-use-tools-help" bind:checked={form.useTools} />
            </Field.Field>
          </Field.Group>
        </div>
        {#if form.limitsMissing}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Description>
              {m.chat_providers_limits_required({
                minContextWindow: formatCount(1_024),
                minOutputLimit: formatCount(16),
              })}
            </Alert.Description>
          </Alert.Root>
        {/if}
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
