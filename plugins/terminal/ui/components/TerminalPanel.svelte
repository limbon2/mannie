<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import * as AlertDialog from '@mannie/plugin-sdk/components/alert-dialog';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Empty from '@mannie/plugin-sdk/components/empty';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import { Kbd } from '@mannie/plugin-sdk/components/kbd';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import ChevronDownIcon from '@mannie/plugin-sdk/icons/chevron-down';
  import ChevronUpIcon from '@mannie/plugin-sdk/icons/chevron-up';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import ListIcon from '@mannie/plugin-sdk/icons/list';
  import SearchIcon from '@mannie/plugin-sdk/icons/search';
  import SquareTerminalIcon from '@mannie/plugin-sdk/icons/square-terminal';
  import { call } from '@mannie/plugin-sdk/ipc';
  import { mention } from '@mannie/plugin-sdk/message-box';
  import type { SidePanelProps } from '@mannie/plugin-sdk/slots';
  import { onMount } from 'svelte';
  import { on } from 'svelte/events';

  import { commands, type HostChoice, type TerminalSummary } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { TerminalViewState } from '../state/terminal-view.svelte.js';
  import { terminals } from '../state/terminals.svelte.js';
  import type { TerminalAddress, TerminalFailure } from '../types/terminal.js';
  import { endingMessage, phaseLabel, terminalFailureMessage, terminalName } from '../utils/terminal-messages.js';
  import { parsePanelArgument } from '../utils/terminal-panel.js';

  import BlockList from './BlockList.svelte';

  // A terminal in its side panel (plugins/terminal/docs/README.md, opening
  // and using terminals): the terminal, a toolbar with Find, the list of
  // commands and End, which asks first while a command runs, and what to do
  // once the terminal ended. The view follows the terminal only while the
  // panel's tab shows and the window is visible, and starts from a snapshot
  // when it shows again. Ctrl+` moves the focus from the terminal to the
  // toolbar, so the keyboard never stays in the terminal (WCAG 2.1.2).
  let { argument, close, setLabel }: SidePanelProps = $props();

  let element = $state<HTMLElement | null>(null);
  let toolbar = $state<HTMLElement | null>(null);
  let finding = $state(false);
  let findText = $state('');
  let showCommands = $state(false);
  let confirmingEnd = $state(false);
  let notice = $state<string | null>(null);
  let failure = $state<TerminalFailure | null>(null);
  /**
   * Whether the panel moved the focus into its terminal yet. It does once,
   * when the terminal first shows, unless the panel opened behind another
   * tab, as on a reload: showing that tab later leaves the focus on the tab.
   */
  let hasFocused = false;
  /** Whether the panel learned yet whether its tab showed when it opened. */
  let hasLearnedTab = false;

  const address = $derived(parsePanelArgument(argument));
  const summary = $derived(address === null ? undefined : summaryOf(address));
  // A view opens xterm.js with the settings of its time, so other settings
  // make a new view.
  const view = $derived(address === null ? null : new TerminalViewState(address, terminals.settings));
  const ended = $derived(view?.ending ?? null);
  const shownFailure = $derived(failure ?? view?.failure ?? view?.followFailure ?? null);
  const isPlain = $derived(isPlainShell());
  const hasSelection = $derived((view?.selectedText ?? '').trim() !== '');

  /** The terminal as the conversation's list shows it. */
  function summaryOf(terminal: TerminalAddress): TerminalSummary | undefined {
    return terminals.of(terminal.conversationId).find((candidate) => candidate.number === terminal.terminal);
  }

  /** Whether the shell reached its prompt without marking commands. */
  function isPlainShell(): boolean {
    if (view === null) {
      return false;
    }
    const atPrompt = view.phase === 'prompt';
    return atPrompt && !view.marking;
  }

  /** Ctrl+` in the terminal moves the focus to the toolbar. */
  function leaveTerminal(event: KeyboardEvent): void {
    const withCommandKey = event.ctrlKey || event.metaKey;
    const isTerminalKey = withCommandKey && event.code === 'Backquote';
    if (!isTerminalKey) {
      return;
    }
    const inTerminal = element?.contains(event.target as Node) ?? false;
    if (inTerminal) {
      event.preventDefault();
      event.stopPropagation();
      toolbar?.querySelector<HTMLElement>('button')?.focus();
    }
  }

  /** Scrolls the terminal to a command of the list and marks it. */
  function showBlock(block: number): void {
    view?.reveal({ block, lines: null });
  }

  function find(direction: 'next' | 'previous'): void {
    if (findText !== '') {
      view?.find(findText, direction);
    }
  }

  function closeFind(): void {
    finding = false;
    findText = '';
    view?.clearFind();
    view?.focus();
  }

  function handleFindKey(event: KeyboardEvent): void {
    if (event.key === 'Enter') {
      find(event.shiftKey ? 'previous' : 'next');
    } else if (event.key === 'Escape') {
      closeFind();
    }
  }

  async function endTerminal(): Promise<void> {
    if (address === null) {
      return;
    }
    confirmingEnd = false;
    const outcome = await call(() =>
      commands.endTerminal({ conversationId: address.conversationId, terminal: address.terminal }),
    );
    failure = outcome.ok ? null : outcome.failure;
  }

  function askToEnd(): void {
    if (view?.phase === 'running') {
      confirmingEnd = true;
    } else {
      void endTerminal();
    }
  }

  async function removeAndClose(): Promise<void> {
    await endTerminal();
    close();
  }

  /** Opens a new terminal where this one ran, in a tab of its own. */
  async function startAgain(): Promise<void> {
    if (address === null) {
      return;
    }
    if (summary === undefined) {
      return;
    }
    const host: HostChoice =
      summary.host.kind === 'remote'
        ? { key: summary.host.key, kind: 'plugin', plugin: summary.host.plugin }
        : { kind: 'this_computer', shell: summary.host.program };
    failure = await terminals.open(address.conversationId, host, null);
  }

  async function mentionSelection(): Promise<void> {
    if (address === null) {
      return;
    }
    const selected = view?.selectedText ?? '';
    if (selected.trim() === '') {
      return;
    }
    const outcome = await call(() =>
      commands.mentionSelection({ conversationId: address.conversationId, terminal: address.terminal, text: selected }),
    );
    if (!outcome.ok) {
      failure = outcome.failure;
      return;
    }
    const added = mention(address.conversationId, { entry: outcome.data, provider: 'terminal' });
    notice = added === 'added' ? null : m.terminal_mention_not_added();
  }

  /** Tells the view whether the panel's tab shows it, and learns whether it showed when the panel opened. */
  function tabShown(current: TerminalViewState, shown: boolean): void {
    if (!hasLearnedTab) {
      hasLearnedTab = true;
      if (!shown) {
        hasFocused = true;
      }
    }
    current.setTabShown(shown);
  }

  $effect(() => {
    if (address !== null) {
      return terminals.watch(address.conversationId);
    }
  });

  $effect(() => {
    if (summary !== undefined) {
      setLabel(terminalName(summary), true);
    }
  });

  $effect(() => {
    const current = view;
    const container = element;
    if (current === null) {
      return;
    }
    if (container === null) {
      return;
    }
    current.setWindowShown(document.visibilityState === 'visible');
    current.attach(container);
    // The side panel area hides the panels of other tabs, so the terminal's
    // element stops intersecting the window while another tab shows.
    const tab = new IntersectionObserver((entries) => {
      const last = entries.at(-1);
      if (last !== undefined) {
        tabShown(current, last.isIntersecting);
      }
    });
    tab.observe(container);
    const resize = new ResizeObserver(() => {
      current.fit();
    });
    resize.observe(container);
    return () => {
      tab.disconnect();
      resize.disconnect();
      current.dispose();
    };
  });

  // A panel takes the focus into its terminal once the terminal first shows,
  // as Ctrl+` and the control promise; Ctrl+` moves it out again.
  $effect(() => {
    if (view?.ready !== true) {
      return;
    }
    if (!hasFocused) {
      hasFocused = true;
      view.focus();
    }
  });

  // A command that a citation or a card asked for shows once it is there.
  $effect(() => {
    if (view === null) {
      return;
    }
    const reveal = terminals.hasReveal(argument) ? terminals.takeReveal(argument) : undefined;
    if (reveal !== undefined) {
      view.reveal(reveal);
    }
  });

  onMount(() =>
    on(document, 'visibilitychange', () => {
      view?.setWindowShown(document.visibilityState === 'visible');
    }),
  );
</script>

{#if view === null}
  <Empty.Root>
    <Empty.Header>
      <Empty.Title>{m.terminal_panel_unknown()}</Empty.Title>
    </Empty.Header>
  </Empty.Root>
{:else}
  <div class="flex min-h-0 flex-1 flex-col" role="presentation" onkeydowncapture={leaveTerminal}>
    <div
      bind:this={toolbar}
      class="flex shrink-0 items-center gap-1 border-b px-2 py-1"
      aria-label={m.terminal_toolbar_label()}
      role="toolbar"
    >
      <Button aria-pressed={finding} size="sm" variant="ghost" onclick={() => (finding = !finding)}>
        <SearchIcon data-icon="inline-start" />
        {m.terminal_find()}
      </Button>
      <Button aria-pressed={showCommands} size="sm" variant="ghost" onclick={() => (showCommands = !showCommands)}>
        <ListIcon data-icon="inline-start" />
        {m.terminal_commands()}
      </Button>
      {#if hasSelection}
        <Button size="sm" variant="ghost" onclick={() => void mentionSelection()}>
          {m.terminal_mention_selection_action()}
        </Button>
      {/if}
      <span class="ms-auto truncate text-xs text-muted-foreground" data-content>
        {view.state ?? phaseLabel(view.phase)}
      </span>
      {#if ended === null}
        <Button size="sm" variant="ghost" onclick={askToEnd}>{m.terminal_end()}</Button>
      {/if}
    </div>
    {#if finding}
      <div class="flex shrink-0 items-center gap-1 border-b px-2 py-1">
        <Input
          class="h-8"
          aria-label={m.terminal_find_label()}
          placeholder={m.terminal_find_placeholder()}
          bind:value={findText}
          onkeydown={handleFindKey}
        />
        <Button aria-label={m.terminal_find_previous()} size="icon-sm" variant="ghost" onclick={() => find('previous')}>
          <ChevronUpIcon />
        </Button>
        <Button aria-label={m.terminal_find_next()} size="icon-sm" variant="ghost" onclick={() => find('next')}>
          <ChevronDownIcon />
        </Button>
        <Button size="sm" variant="ghost" onclick={closeFind}>{m.terminal_find_close()}</Button>
      </div>
    {/if}
    {#if isPlain}
      <p class="shrink-0 border-b px-3 py-1 text-xs text-muted-foreground">{m.terminal_plain_shell()}</p>
    {/if}
    {#if shownFailure !== null}
      <Alert.Root class="m-2 w-auto" variant="destructive">
        <CircleAlertIcon />
        <Alert.Description>{terminalFailureMessage(shownFailure)}</Alert.Description>
      </Alert.Root>
    {/if}
    {#if notice !== null}
      <p class="shrink-0 px-3 py-1 text-xs" role="status">{notice}</p>
    {/if}
    <div class="relative flex min-h-0 flex-1">
      <div
        bind:this={element}
        class="min-h-0 min-w-0 flex-1 bg-background p-1"
        aria-label={m.terminal_region_label()}
        data-terminal-view=""
        role="region"
      ></div>
      {#if !view.ready}
        <div
          class="absolute inset-0 flex items-center justify-center gap-2 text-sm text-muted-foreground"
          role="status"
        >
          <Spinner />
          {m.terminal_loading()}
        </div>
      {/if}
      {#if showCommands}
        <div class="w-72 shrink-0 overflow-y-auto border-s">
          <BlockList address={view.address} blocks={view.blocks} onShow={showBlock} />
        </div>
      {/if}
    </div>
    {#if ended !== null}
      <div class="flex shrink-0 flex-wrap items-center gap-2 border-t px-3 py-2 text-sm" role="status">
        <SquareTerminalIcon class="size-4" />
        <span>{endingMessage(ended)}</span>
        <Button class="ms-auto" size="sm" variant="outline" onclick={() => void startAgain()}>
          {m.terminal_start_again()}
        </Button>
        <Button size="sm" variant="ghost" onclick={() => void removeAndClose()}>{m.terminal_remove()}</Button>
      </div>
    {/if}
    <p class="shrink-0 px-3 py-1 text-xs text-muted-foreground">
      {m.terminal_leave_hint_start()}
      <Kbd>{m.terminal_key()}</Kbd>
      {m.terminal_leave_hint_end()}
    </p>
  </div>
  <AlertDialog.Root bind:open={confirmingEnd}>
    <AlertDialog.Content>
      <AlertDialog.Header>
        <AlertDialog.Title>{m.terminal_end_title()}</AlertDialog.Title>
        <AlertDialog.Description>{m.terminal_end_description()}</AlertDialog.Description>
      </AlertDialog.Header>
      <AlertDialog.Footer>
        <AlertDialog.Cancel>{m.terminal_cancel()}</AlertDialog.Cancel>
        <AlertDialog.Action onclick={() => void endTerminal()}>{m.terminal_end()}</AlertDialog.Action>
      </AlertDialog.Footer>
    </AlertDialog.Content>
  </AlertDialog.Root>
{/if}
