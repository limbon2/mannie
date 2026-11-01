<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import SquareTerminalIcon from '@mannie/plugin-sdk/icons/square-terminal';
  import type { ToolKindDetailsProps } from '@mannie/plugin-sdk/slots';

  import { m } from '../generated/messages.js';
  import { terminals } from '../state/terminals.svelte.js';
  import type { Reveal } from '../types/terminal.js';

  // The details of a call of `run_in_terminal` (ADR-0039, decision 1): Show
  // in terminal, which opens the terminal's panel at the command, while the
  // terminal still exists in this window.
  let { call, conversationId }: ToolKindDetailsProps = $props();

  const target = $derived(targetOf());

  /** A whole number of the card's data, or `null`. */
  function numberOf(key: string): number | null {
    const value = call.data?.[key];
    if (typeof value !== 'number') {
      return null;
    }
    return Number.isSafeInteger(value) ? value : null;
  }

  /** The terminal and command that the call ran, while the terminal exists. */
  function targetOf(): { reveal: Reveal; terminal: number } | null {
    const terminal = numberOf('terminal');
    const block = numberOf('block');
    const session = call.data?.session;
    if (terminal === null) {
      return null;
    }
    if (block === null) {
      return null;
    }
    const exists = terminals.of(conversationId).some((summary) => {
      const isNumber = summary.number === terminal;
      const isSession = summary.session === session;
      return isNumber && isSession;
    });
    return exists ? { reveal: { block, lines: null }, terminal } : null;
  }

  function show(): void {
    if (target !== null) {
      void terminals.reveal(conversationId, target.terminal, target.reveal);
    }
  }

  $effect(() => terminals.watch(conversationId));
</script>

{#if target !== null}
  <Button class="self-start" size="sm" variant="outline" onclick={show}>
    <SquareTerminalIcon data-icon="inline-start" />
    {m.terminal_show_in_terminal()}
  </Button>
{/if}
