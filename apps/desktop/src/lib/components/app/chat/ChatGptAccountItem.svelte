<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
  import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
  import { onMount } from 'svelte';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Badge } from '#lib/components/ui/badge/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Item from '#lib/components/ui/item/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import type { ChatGptAccountView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { type LinkFailure, linkFailureMessage } from '#lib/sdk/links.js';
  import { chatgptAccount } from '#lib/state/chatgpt-account.svelte.js';
  import type { ChatGptFailure } from '#lib/types/chat.js';
  import { chatgptFailureMessage } from '#lib/utils/chat-errors.js';

  import ChatGptNoticeDialog from './ChatGptNoticeDialog.svelte';
  import ChatGptSignInButton from './ChatGptSignInButton.svelte';

  // The ChatGPT account above the provider profiles (docs/features/
  // chatgpt-plan.md, the ChatGPT account): what the plan does and "Continue
  // with ChatGPT" before the first sign-in; the progress of a sign-in with
  // "Open the sign-in page again" and "Cancel"; the signed-in account with
  // "Manage usage" and "Sign out"; and a signed-out account, or one whose
  // session ended, with the sign-in again and "Use another account". A
  // failed sign-in says what failed and what the owner can do, and OpenAI's
  // notice shows once after the first sign-in.
  let actionFailure = $state<ChatGptFailure | null>(null);
  let linkFailure = $state<LinkFailure | null>(null);
  /** Whether the latest sign-out ended without OpenAI's confirmation that it revoked the session. */
  let unconfirmedSignOut = $state(false);

  const view = $derived(chatgptAccount.account);
  const step = $derived(view?.signIn.step ?? 'idle');
  const busy = $derived(chatgptAccount.pending !== null);
  const progress = $derived(step === 'finishing' ? m.chat_chatgpt_finishing() : m.chat_chatgpt_waiting());
  /** Why the latest sign-in failed, while no account is signed in and no sign-in runs. */
  const signInFailure = $derived(
    chatgptAccount.signedIn || chatgptAccount.signingIn ? null : chatgptAccount.signInFailure,
  );
  /** Whether to say that OpenAI did not confirm the sign-out, until an account signs in again. */
  const showsUnconfirmedSignOut = $derived(unconfirmedSignOut && !chatgptAccount.signedIn);

  /** What the item says about the account in its state. */
  function stateLine(account: ChatGptAccountView): string {
    const { email } = account;
    switch (account.state) {
      case 'sign_in_needed': {
        return email === null ? m.chat_chatgpt_sign_in_needed() : m.chat_chatgpt_sign_in_needed_as({ email });
      }
      case 'signed_in': {
        return email === null ? m.chat_chatgpt_signed_in() : m.chat_chatgpt_signed_in_as({ email });
      }
      case 'signed_out': {
        if (!account.known) {
          return m.chat_chatgpt_description();
        }
        return email === null ? m.chat_chatgpt_signed_out() : m.chat_chatgpt_signed_out_as({ email });
      }
    }
  }

  function clearFailures(): void {
    actionFailure = null;
    linkFailure = null;
  }

  async function signIn(anotherAccount: boolean): Promise<void> {
    clearFailures();
    unconfirmedSignOut = false;
    // A sign-in that does not start says why through the account's sign-in failure.
    await chatgptAccount.signIn(anotherAccount);
  }

  async function reopen(): Promise<void> {
    clearFailures();
    actionFailure = await chatgptAccount.reopenPage();
  }

  async function cancel(): Promise<void> {
    clearFailures();
    actionFailure = await chatgptAccount.cancel();
  }

  async function signOut(): Promise<void> {
    clearFailures();
    const outcome = await chatgptAccount.signOut();
    if (outcome.ok) {
      unconfirmedSignOut = !outcome.data.revoked;
    } else {
      actionFailure = outcome.failure;
    }
  }

  async function manageUsage(): Promise<void> {
    clearFailures();
    linkFailure = await chatgptAccount.manageUsage();
  }

  onMount(() => {
    chatgptAccount.start();
  });
</script>

<div class="flex flex-col gap-3">
  {#if chatgptAccount.status === 'loading'}
    <div aria-busy="true">
      <span class="sr-only" role="status">{m.chat_chatgpt_loading()}</span>
      <Skeleton class="h-16 w-full" />
    </div>
  {:else if chatgptAccount.status === 'error'}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.chat_chatgpt_load_failed_title()}</Alert.Title>
      <Alert.Description>
        {chatgptAccount.failure === null ? '' : chatgptFailureMessage(chatgptAccount.failure)}
      </Alert.Description>
      <Alert.Action>
        <Button size="sm" variant="outline" onclick={() => chatgptAccount.retry()}>{m.chat_try_again()}</Button>
      </Alert.Action>
    </Alert.Root>
  {:else if view !== null}
    <Item.Root data-chatgpt-account={view.state} data-chatgpt-sign-in-step={step} variant="outline">
      <Item.Content class="min-w-0">
        <Item.Title>
          <h3>{m.chat_chatgpt_account()}</h3>
          {#if view.state === 'signed_in'}
            <Badge variant="secondary">{m.chat_chatgpt_using_plan()}</Badge>
          {/if}
        </Item.Title>
        <Item.Description class="line-clamp-none text-start">{stateLine(view)}</Item.Description>
        {#if chatgptAccount.signingIn}
          <p class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
            <Spinner aria-label={progress} />
            <span>{progress}</span>
          </p>
        {/if}
      </Item.Content>
      <Item.Actions class="flex-wrap">
        {#if chatgptAccount.signingIn}
          {#if step === 'waiting_for_browser'}
            <Button disabled={busy} size="sm" variant="outline" onclick={reopen}>{m.chat_chatgpt_reopen()}</Button>
          {/if}
          <Button disabled={busy} size="sm" variant="outline" onclick={cancel}>{m.chat_cancel()}</Button>
        {:else if view.state === 'signed_in'}
          <Button size="sm" variant="outline" onclick={manageUsage}>
            {m.chat_chatgpt_manage_usage()}
            <ExternalLinkIcon data-icon="inline-end" />
          </Button>
          <Button disabled={busy} size="sm" variant="outline" onclick={signOut}>{m.chat_chatgpt_sign_out()}</Button>
        {:else}
          {#if view.known}
            <Button disabled={busy} size="sm" variant="outline" onclick={() => void signIn(true)}>
              {m.chat_chatgpt_another_account()}
            </Button>
          {/if}
          <ChatGptSignInButton disabled={busy} size="sm" onclick={() => void signIn(false)} />
        {/if}
      </Item.Actions>
    </Item.Root>
    {#if signInFailure !== null}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.chat_chatgpt_sign_in_failed_title()}</Alert.Title>
        <Alert.Description>{chatgptFailureMessage(signInFailure)}</Alert.Description>
      </Alert.Root>
    {/if}
    {#if showsUnconfirmedSignOut}
      <Alert.Root>
        <TriangleAlertIcon />
        <Alert.Description>{m.chat_chatgpt_sign_out_unconfirmed()}</Alert.Description>
      </Alert.Root>
    {/if}
  {/if}
  {#if actionFailure !== null}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Description>{chatgptFailureMessage(actionFailure)}</Alert.Description>
    </Alert.Root>
  {/if}
  {#if linkFailure !== null}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Description>{linkFailureMessage(linkFailure)}</Alert.Description>
    </Alert.Root>
  {/if}
</div>

<ChatGptNoticeDialog />
