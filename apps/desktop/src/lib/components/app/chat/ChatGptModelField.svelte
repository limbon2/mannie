<script lang="ts">
  import { onMount, untrack } from 'svelte';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import * as Select from '#lib/components/ui/select/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatgptAccount } from '#lib/state/chatgpt-account.svelte.js';
  import type { ProviderProfileForm } from '#lib/state/provider-profile-form.svelte.js';
  import type { ChatGptFailure } from '#lib/types/chat.js';
  import { chatgptFailureMessage } from '#lib/utils/chat-errors.js';

  import ChatGptSignInButton from './ChatGptSignInButton.svelte';

  // The model of a profile on the ChatGPT plan (docs/features/chatgpt-plan.md,
  // plan profiles): one of the models that the plan offers the signed-in
  // account, chosen by its name. The field reads the catalog whenever the
  // account signs in while it shows, and says so while the account or the
  // catalog loads, while no account is signed in, which it offers to change,
  // and when the catalog is empty or could not be read. An edited profile
  // shows its model meanwhile.
  interface Props {
    form: ProviderProfileForm;
    /** Whether the core refused the model, for the field's `data-invalid`. */
    invalid: true | undefined;
  }

  /** What the field can offer: the models, or why it cannot yet. */
  type FieldState = 'account_failed' | 'catalog_failed' | 'empty' | 'loading' | 'ready' | 'signed_out';

  let { form, invalid }: Props = $props();

  /** The ID of the line under the field that says why no model can be chosen yet. */
  const STATUS_ID = 'profile-plan-model-status';

  let actionFailure = $state<ChatGptFailure | null>(null);

  const catalog = $derived(form.catalog);
  const catalogFailure = $derived(catalog.status === 'error' ? catalog.failure : null);
  const models = $derived(catalog.status === 'ready' ? catalog.models : []);
  const items = $derived(models.map((model) => ({ label: model.displayName, value: model.slug })));
  /** The chosen model's name in the catalog, or its stored name while the catalog does not list it. */
  const chosenName = $derived(models.find((model) => model.slug === form.model)?.displayName ?? form.model);
  const fieldState = $derived.by((): FieldState => {
    if (chatgptAccount.status === 'loading') {
      return 'loading';
    }
    if (chatgptAccount.status === 'error') {
      return 'account_failed';
    }
    if (!chatgptAccount.signedIn) {
      return 'signed_out';
    }
    switch (catalog.status) {
      case 'error': {
        return 'catalog_failed';
      }
      case 'idle':
      case 'loading': {
        return 'loading';
      }
      case 'ready': {
        return catalog.models.length === 0 ? 'empty' : 'ready';
      }
    }
  });
  const busy = $derived(chatgptAccount.pending !== null);
  const waiting = $derived(chatgptAccount.account?.signIn.step === 'waiting_for_browser');
  const progress = $derived(waiting ? m.chat_chatgpt_waiting() : m.chat_chatgpt_finishing());
  const invalidField = $derived(invalid ?? (form.modelMissing ? true : undefined));

  async function signIn(): Promise<void> {
    actionFailure = null;
    // A sign-in that does not start says why through the account's sign-in failure.
    await chatgptAccount.signIn(false);
  }

  async function reopen(): Promise<void> {
    actionFailure = await chatgptAccount.reopenPage();
  }

  async function cancel(): Promise<void> {
    actionFailure = await chatgptAccount.cancel();
  }

  // The catalog belongs to the account, so the form reads it again each time
  // an account signs in.
  $effect(() => {
    const current = form;
    if (chatgptAccount.signedIn) {
      untrack(() => {
        void current.loadModels();
      });
    }
  });

  onMount(() => {
    chatgptAccount.start();
  });
</script>

<Field.Field data-invalid={invalidField}>
  <Field.Label for="profile-plan-model">{m.chat_providers_field_model()}</Field.Label>
  <Select.Root
    disabled={fieldState !== 'ready'}
    {items}
    type="single"
    bind:value={() => form.model, (slug: string) => void form.chooseModel(slug)}
  >
    <Select.Trigger
      id="profile-plan-model"
      class="w-full"
      aria-describedby={fieldState === 'ready' ? undefined : STATUS_ID}
      aria-invalid={invalidField}
    >
      {#if form.model === ''}
        <span class="text-muted-foreground">{m.chat_chatgpt_model_placeholder()}</span>
      {:else}
        <span class="truncate" data-content>{chosenName}</span>
      {/if}
    </Select.Trigger>
    <Select.Content>
      {#each models as model (model.slug)}
        <Select.Item label={model.displayName} value={model.slug}>
          <span data-content>{model.displayName}</span>
        </Select.Item>
      {/each}
    </Select.Content>
  </Select.Root>
  {#if fieldState === 'loading'}
    <p id={STATUS_ID} class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
      <Spinner aria-label={m.chat_chatgpt_models_loading()} />
      <span>{m.chat_chatgpt_models_loading()}</span>
    </p>
  {:else if fieldState === 'account_failed'}
    <div class="flex flex-col items-start gap-2">
      <p id={STATUS_ID} class="text-sm text-destructive" role="alert">
        {chatgptAccount.failure === null ? '' : chatgptFailureMessage(chatgptAccount.failure)}
      </p>
      <Button size="sm" variant="outline" onclick={() => chatgptAccount.retry()}>{m.chat_try_again()}</Button>
    </div>
  {:else if fieldState === 'signed_out'}
    <div class="flex flex-col items-start gap-2">
      <p id={STATUS_ID} class="text-sm text-muted-foreground">{m.chat_chatgpt_models_signed_out()}</p>
      {#if chatgptAccount.signingIn}
        <p class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
          <Spinner aria-label={progress} />
          <span>{progress}</span>
        </p>
        <div class="flex flex-wrap gap-2">
          {#if waiting}
            <Button disabled={busy} size="sm" variant="outline" onclick={reopen}>{m.chat_chatgpt_reopen()}</Button>
          {/if}
          <Button disabled={busy} size="sm" variant="outline" onclick={cancel}>{m.chat_cancel()}</Button>
        </div>
      {:else}
        <ChatGptSignInButton disabled={busy} size="sm" onclick={() => void signIn()} />
        {#if chatgptAccount.signInFailure !== null}
          <p class="text-sm text-destructive" role="alert">{chatgptFailureMessage(chatgptAccount.signInFailure)}</p>
        {/if}
      {/if}
      {#if actionFailure !== null}
        <p class="text-sm text-destructive" role="alert">{chatgptFailureMessage(actionFailure)}</p>
      {/if}
    </div>
  {:else if catalogFailure !== null}
    <div class="flex flex-col items-start gap-2">
      <p id={STATUS_ID} class="text-sm text-destructive" role="alert">
        {m.chat_chatgpt_models_failed({ reason: chatgptFailureMessage(catalogFailure) })}
      </p>
      <Button size="sm" variant="outline" onclick={() => void form.loadModels()}>{m.chat_try_again()}</Button>
    </div>
  {:else if fieldState === 'empty'}
    <p id={STATUS_ID} class="text-sm text-muted-foreground">{m.chat_chatgpt_models_empty()}</p>
  {:else if form.defaultsApplied}
    <Field.Description>{m.chat_providers_defaults_applied()}</Field.Description>
  {/if}
  {#if form.modelMissing}
    <Field.Error>{m.chat_chatgpt_model_missing()}</Field.Error>
  {/if}
</Field.Field>
