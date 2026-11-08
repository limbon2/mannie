<script lang="ts">
  import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatgptAccount } from '#lib/state/chatgpt-account.svelte.js';
  import type { ChatGptFailure } from '#lib/types/chat.js';
  import { chatgptFailureMessage } from '#lib/utils/chat-errors.js';

  // The notice that OpenAI asks apps to show once, after the first sign-in
  // with the ChatGPT plan, in OpenAI's words (docs/features/chatgpt-plan.md,
  // the ChatGPT account). "Got it" records that the owner saw it, and the
  // account's update closes it; when the record fails, the notice says why
  // and stays. Escape closes it at once and records the same, and a notice
  // whose record failed shows again next time.
  let failure = $state<ChatGptFailure | null>(null);
  /** Whether the owner closed the notice with Escape. */
  let dismissed = $state(false);

  const open = $derived(chatgptAccount.noticeDue && !dismissed);

  async function acknowledge(): Promise<void> {
    failure = await chatgptAccount.acknowledgeNotice();
  }

  function setOpen(next: boolean): void {
    if (next) {
      return;
    }
    dismissed = true;
    void acknowledge();
  }
</script>

<AlertDialog.Root bind:open={() => open, setOpen}>
  <AlertDialog.Content data-chatgpt-notice="">
    <AlertDialog.Header>
      <AlertDialog.Title>{m.chat_chatgpt_notice_title()}</AlertDialog.Title>
      <AlertDialog.Description>{m.chat_chatgpt_notice_description()}</AlertDialog.Description>
    </AlertDialog.Header>
    {#if failure !== null}
      <p class="text-sm text-destructive" role="alert">{chatgptFailureMessage(failure)}</p>
    {/if}
    <AlertDialog.Footer>
      <AlertDialog.Action disabled={chatgptAccount.pending === 'acknowledge'} onclick={() => void acknowledge()}>
        {m.chat_chatgpt_notice_confirm()}
      </AlertDialog.Action>
    </AlertDialog.Footer>
  </AlertDialog.Content>
</AlertDialog.Root>
