<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
  import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { type LinkFailure, linkFailureMessage } from '#lib/sdk/links.js';
  import { chatActions } from '#lib/state/chat-actions.svelte.js';
  import { chatgptAccount } from '#lib/state/chatgpt-account.svelte.js';
  import { getConversationContext } from '#lib/state/conversation-context.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import type { PartRendererProps } from '#lib/types/shell-slots.js';
  import { chatFailureMessage, runFailureMessage } from '#lib/utils/chat-errors.js';
  import { PROVIDER_SETTINGS_SECTION } from '#lib/utils/chat-urls.js';
  import { openSettings } from '#lib/utils/settings-sections.js';

  // A failed reply: what failed, what the owner can do, and Retry for the
  // latest turn, unless the conversation shows read-only. The ChatGPT plan's
  // usage limit shows OpenAI's words with "Manage usage" first, and a failure
  // that needs the ChatGPT account offers its settings (docs/features/
  // chatgpt-plan.md, in conversations).
  let { conversationId, latest, part }: PartRendererProps = $props();

  const context = getConversationContext();

  let retrying = $state(false);
  let retryFailure = $state<ChatFailure | null>(null);
  let linkFailure = $state<LinkFailure | null>(null);

  const failure = $derived(part.content.kind === 'error' ? part.content.failure : null);
  const message = $derived(failure === null ? '' : runFailureMessage(failure));
  const usageLimit = $derived(failure?.code === 'chatgpt_usage_limit_reached');
  /** Whether the owner fixes the failure in the ChatGPT account: a new sign-in, or the account that signed in meanwhile. */
  const needsAccount = $derived.by(() => {
    const code = failure?.code;
    const signInNeeded = code === 'chatgpt_sign_in_needed';
    const accountChanged = code === 'chatgpt_account_changed';
    return signInNeeded || accountChanged;
  });
  const canRetry = $derived(latest && !context.readOnly);

  async function retry(): Promise<void> {
    retrying = true;
    retryFailure = await chatActions.retry(conversationId);
    retrying = false;
  }

  async function manageUsage(): Promise<void> {
    linkFailure = await chatgptAccount.manageUsage();
  }
</script>

<Alert.Root variant="destructive">
  <CircleAlertIcon />
  {#if usageLimit}
    <Alert.Title>{m.chat_chatgpt_usage_limit_title()}</Alert.Title>
  {:else}
    <Alert.Title>{m.chat_reply_failed()}</Alert.Title>
  {/if}
  <Alert.Description>
    {#if usageLimit}
      <p>{m.chat_chatgpt_usage_limit_description()}</p>
    {:else}
      <p>{message}</p>
    {/if}
    {#if retryFailure !== null}
      <p>{chatFailureMessage(retryFailure)}</p>
    {/if}
    {#if linkFailure !== null}
      <p>{linkFailureMessage(linkFailure)}</p>
    {/if}
  </Alert.Description>
  {#if usageLimit || needsAccount}
    <!-- Two actions do not fit the alert's corner, so they sit under the text, the first one the primary. -->
    <div class="col-start-2 mt-2 flex flex-wrap gap-2">
      {#if usageLimit}
        <Button size="sm" onclick={manageUsage}>
          {m.chat_chatgpt_manage_usage()}
          <ExternalLinkIcon data-icon="inline-end" />
        </Button>
      {:else}
        <Button size="sm" variant="outline" onclick={() => void openSettings(PROVIDER_SETTINGS_SECTION)}>
          {m.chat_chatgpt_open_settings()}
        </Button>
      {/if}
      {#if canRetry}
        <Button disabled={retrying} size="sm" variant="outline" onclick={retry}>
          <RotateCcwIcon data-icon="inline-start" />
          {m.chat_retry()}
        </Button>
      {/if}
    </div>
  {:else if canRetry}
    <Alert.Action>
      <Button disabled={retrying} size="sm" variant="outline" onclick={retry}>
        <RotateCcwIcon data-icon="inline-start" />
        {m.chat_retry()}
      </Button>
    </Alert.Action>
  {/if}
</Alert.Root>
