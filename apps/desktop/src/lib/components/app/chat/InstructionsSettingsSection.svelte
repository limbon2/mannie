<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import { onMount } from 'svelte';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import { Textarea } from '#lib/components/ui/textarea/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatSettings } from '#lib/state/chat-settings.svelte.js';
  import { chatFailureMessage, formatCount } from '#lib/utils/chat-errors.js';
  import { countCharacters, MAX_INSTRUCTIONS_CHARACTERS } from '#lib/utils/chat-instructions.js';

  // The owner's instructions, which apply to every conversation from its next
  // reply (docs/features/chat/conversations.md, the owner's instructions).

  let draft = $state('');
  let saved = $state(false);

  const characters = $derived(countCharacters(draft));
  const tooLong = $derived(characters > MAX_INSTRUCTIONS_CHARACTERS);
  const changed = $derived(draft !== chatSettings.instructions);

  async function save(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    saved = await chatSettings.save(draft);
  }

  onMount(() => {
    void chatSettings.load().then(() => {
      draft = chatSettings.instructions;
    });
  });
</script>

{#if chatSettings.status === 'loading'}
  <div class="flex flex-col gap-2" aria-busy="true">
    <Skeleton class="h-4 w-32" />
    <Skeleton class="h-32 w-full" />
  </div>
{:else if chatSettings.readFailure !== null}
  <Alert.Root variant="destructive">
    <CircleAlertIcon />
    <Alert.Title>{m.shell_settings_load_failed_title()}</Alert.Title>
    <Alert.Description>{chatFailureMessage(chatSettings.readFailure)}</Alert.Description>
    <Alert.Action>
      <Button size="sm" variant="outline" onclick={() => void chatSettings.load()}>{m.chat_try_again()}</Button>
    </Alert.Action>
  </Alert.Root>
{:else}
  <form class="flex flex-col gap-3" onsubmit={save}>
    <Field.Field data-invalid={tooLong ? true : undefined}>
      <Field.Label for="chat-instructions">{m.chat_instructions_label()}</Field.Label>
      <Textarea
        id="chat-instructions"
        class="min-h-32"
        aria-describedby="chat-instructions-size"
        aria-invalid={tooLong}
        bind:value={draft}
        oninput={() => (saved = false)}
      />
      <Field.Description id="chat-instructions-size">
        {m.chat_instructions_size({ max: formatCount(MAX_INSTRUCTIONS_CHARACTERS), used: formatCount(characters) })}
      </Field.Description>
    </Field.Field>
    {#if chatSettings.saveFailure !== null}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.shell_settings_save_failed_title()}</Alert.Title>
        <Alert.Description>{chatFailureMessage(chatSettings.saveFailure)}</Alert.Description>
      </Alert.Root>
    {/if}
    <div class="flex items-center gap-3">
      <Button disabled={chatSettings.saving || !changed || tooLong} type="submit">{m.chat_save()}</Button>
      <span class="text-sm text-muted-foreground" role="status">{saved ? m.chat_instructions_saved() : ''}</span>
    </div>
  </form>
{/if}
