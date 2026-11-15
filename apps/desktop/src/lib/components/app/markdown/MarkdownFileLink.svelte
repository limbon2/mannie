<script lang="ts">
  import type { Snippet } from 'svelte';
  import { onDestroy } from 'svelte';

  import * as ContextMenu from '#lib/components/ui/context-menu/index.js';
  import type { FileLinkRef } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { getConversationContext, hasConversationContext } from '#lib/state/conversation-context.js';
  import { FileLinkState } from '#lib/state/file-link.svelte.js';
  import { fileLinkFailureMessage } from '#lib/utils/file-link-messages.js';

  let { children, path }: { children: Snippet; path: string } = $props();
  const conversation = hasConversationContext() ? getConversationContext() : null;
  const statusId = $props.id();
  let trigger = $state<HTMLElement | null>(null);
  let restoreFocus = false;
  const reference: FileLinkRef = $derived({ conversationId: conversation?.conversationId ?? null, path });
  const navigation = $derived(new FileLinkState(reference));
  const busy = $derived(navigation.pending !== null);
  const failed = $derived(navigation.failure !== null);
  const hasStatus = $derived(failed || busy);
  const unresolved = $derived(navigation.resolved === null);
  const menuUnavailable = $derived(unresolved || busy);
  const pendingMessage = $derived(
    navigation.pending === 'checking' ? m.markdown_file_checking() : m.markdown_file_pending(),
  );
  const application = $derived(
    navigation.resolved?.application === 'file_explorer' ? m.markdown_file_explorer() : m.markdown_file_manager(),
  );

  function menuChanged(open: boolean): void {
    if (open) {
      void navigation.resolve();
    }
  }

  function openFromMenu(): void {
    restoreFocus = true;
    void navigation.open();
  }

  function menuTransitionComplete(open: boolean): void {
    if (!open && restoreFocus) {
      restoreFocus = false;
      trigger?.focus();
    }
  }

  function keyboardMenu(event: KeyboardEvent): void {
    const contextKey = event.key === 'ContextMenu';
    const shiftHeld = event.shiftKey;
    const f10 = event.key === 'F10';
    const shiftF10 = shiftHeld && f10;
    if (!contextKey && !shiftF10) {
      return;
    }
    event.preventDefault();
    const bounds = trigger?.getBoundingClientRect();
    if (bounds === undefined) {
      return;
    }
    trigger?.dispatchEvent(
      new MouseEvent('contextmenu', {
        bubbles: true,
        cancelable: true,
        clientX: bounds.left,
        clientY: bounds.bottom,
      }),
    );
  }

  onDestroy(() => navigation.reset());
</script>

<ContextMenu.Root onOpenChange={menuChanged} onOpenChangeComplete={menuTransitionComplete}>
  <ContextMenu.Trigger tabindex={0} bind:ref={trigger}>
    {#snippet child({ props })}
      <button
        {...props}
        class="cursor-pointer rounded-sm text-primary underline underline-offset-2 outline-none hover:no-underline focus-visible:ring-2 focus-visible:ring-ring"
        style={undefined}
        aria-busy={busy}
        aria-describedby={hasStatus ? statusId : undefined}
        aria-disabled={busy}
        aria-haspopup="menu"
        data-content
        data-file-link
        type="button"
        onclick={() => void navigation.open()}
        onkeydown={keyboardMenu}
      >
        {@render children()}
      </button>
    {/snippet}
  </ContextMenu.Trigger>
  <ContextMenu.Content
    onCloseAutoFocus={(event: Event) => {
      if (restoreFocus) {
        event.preventDefault();
      }
    }}
    onEscapeKeydown={() => {
      restoreFocus = true;
    }}
  >
    <ContextMenu.Group>
      <ContextMenu.Sub>
        <ContextMenu.SubTrigger disabled={menuUnavailable}>
          {m.markdown_file_open_in()}
        </ContextMenu.SubTrigger>
        <ContextMenu.SubContent>
          <ContextMenu.Group>
            <ContextMenu.Item disabled={busy} onSelect={openFromMenu}>
              {application}
            </ContextMenu.Item>
          </ContextMenu.Group>
        </ContextMenu.SubContent>
      </ContextMenu.Sub>
      {#if busy}
        <ContextMenu.Item disabled>{pendingMessage}</ContextMenu.Item>
      {/if}
    </ContextMenu.Group>
  </ContextMenu.Content>
</ContextMenu.Root>
{#if navigation.failure !== null}
  <span id={statusId} class="ms-1 text-destructive" role="alert">{fileLinkFailureMessage(navigation.failure)}</span>
{:else if busy}
  <span id={statusId} class="ms-1 text-muted-foreground" role="status">{pendingMessage}</span>
{/if}
