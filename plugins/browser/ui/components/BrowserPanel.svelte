<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import * as AlertDialog from '@mannie/plugin-sdk/components/alert-dialog';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Empty from '@mannie/plugin-sdk/components/empty';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import ArrowLeftIcon from '@mannie/plugin-sdk/icons/arrow-left';
  import ArrowRightIcon from '@mannie/plugin-sdk/icons/arrow-right';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import GlobeIcon from '@mannie/plugin-sdk/icons/globe';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';
  import RotateCwIcon from '@mannie/plugin-sdk/icons/rotate-cw';
  import XIcon from '@mannie/plugin-sdk/icons/x';
  import type { SidePanelProps } from '@mannie/plugin-sdk/slots';

  import type { BrowserControl, BrowserCredential, BrowserStatus, BrowserTab } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { BrowserSession } from '../state/browser-session.svelte.js';
  import { browserActivityMessage, browserErrorMessage } from '../utils/browser-messages.js';
  import { browserPanelTarget, labelBrowserTab, openBrowserTab } from '../utils/browser-panel.js';

  import BrowserCredentials from './BrowserCredentials.svelte';
  import BrowserViewport from './BrowserViewport.svelte';

  let { argument, close }: SidePanelProps = $props();
  let draftAddress = $state<string | null>(null);
  let confirmingEnd = $state(false);
  let address = $state<HTMLInputElement | null>(null);
  let focusButton = $state<HTMLButtonElement | null>(null);
  /** The tabs of the last status, to find new ones and changed titles. */
  let knownTabs: ReadonlyMap<string, BrowserTab> | null = null;
  /** The last time focus left a page that the pane has handled. */
  let focusSequence: number | null = null;

  const target = $derived(browserPanelTarget(argument));
  const conversationId = $derived(target.conversationId);
  // A panel that moves to another tab of the conversation keeps its session.
  const session = $derived(new BrowserSession(conversationId));
  const status = $derived(session.status);
  const tab = $derived(status?.tabs.find((candidate) => candidate.id === target.tabId));
  const activeTab = $derived(status?.tabs.find((candidate) => candidate.id === status.activeTab));
  const owner = $derived(status?.control === 'owner');
  const interactive = $derived(owner && !session.busy);
  const activity = $derived(activityMessage(status));
  const ended = $derived(endedMessage(status));

  /** The agent's latest gesture while it has control. */
  function activityMessage(current: BrowserStatus | null): string {
    const latest = current?.control === 'agent' ? current.activity : null;
    return latest === null ? '' : browserActivityMessage(latest.kind);
  }

  /** Why the last session ended on its own, if it did. */
  function endedMessage(current: BrowserStatus | null): string | null {
    const failure = current?.failure ?? null;
    return failure === null ? null : browserErrorMessage(failure.code);
  }

  function control(action: BrowserControl): Promise<boolean> {
    return session.control(action, target.tabId);
  }

  function saveCredential(action: BrowserCredential): Promise<boolean> {
    return session.credential(action);
  }

  function editAddress(event: Event): void {
    if (event.currentTarget instanceof HTMLInputElement) {
      draftAddress = event.currentTarget.value;
    }
  }

  async function navigate(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const url = (draftAddress ?? tab?.url ?? '').trim();
    const canNavigate = interactive && url !== '';
    if (!canNavigate) {
      return;
    }
    if (await control({ kind: 'navigate', url })) {
      draftAddress = null;
    }
  }

  async function showActiveTab(replaceArgument?: string): Promise<void> {
    const opened = activeTab;
    if (opened !== undefined) {
      await openBrowserTab(target.conversationId, opened, { replaceArgument });
    }
  }

  async function start(): Promise<void> {
    if (await control({ kind: 'start' })) {
      await showActiveTab(argument);
    }
  }

  async function openTab(): Promise<void> {
    if (await control({ kind: 'open_tab' })) {
      await showActiveTab(target.tabId === null ? argument : undefined);
    }
  }

  async function closeTab(tabId: string): Promise<void> {
    if (await control({ kind: 'close_tab', tabId })) {
      close();
    }
  }

  async function end(): Promise<void> {
    confirmingEnd = false;
    if (await control({ kind: 'end' })) {
      draftAddress = null;
    }
  }

  /** Opens panels for tabs that the agent or a page opened, and keeps every panel's title. */
  async function followTabs(latest: BrowserStatus): Promise<void> {
    const { conversationId, tabId } = target;
    const previous = knownTabs;
    knownTabs = new Map(latest.tabs.map((candidate) => [candidate.id, candidate]));
    const active = knownTabs.get(latest.activeTab ?? '');
    const missing = tabId === null || !knownTabs.has(tabId);
    const replacePanel = missing && active !== undefined && !session.busy;
    if (replacePanel) {
      await openBrowserTab(conversationId, active, { replaceArgument: argument, takesFocus: false });
      return;
    }
    for (const candidate of latest.tabs) {
      const known = previous?.get(candidate.id);
      const changed = known?.title !== candidate.title || known.url !== candidate.url;
      if (changed) {
        labelBrowserTab(conversationId, candidate);
      }
      const newlyOpened = previous !== null && known === undefined && !session.busy;
      if (newlyOpened) {
        await openBrowserTab(conversationId, candidate, { show: false });
      }
    }
  }

  // The status subscription lives as long as the pane: it is the lease of
  // the conversation's pages (ADR-0063, decision 5).
  $effect(() => {
    const current = session;
    current.follow();
    return () => {
      current.stop();
    };
  });

  $effect(() => {
    const latest = status;
    if (latest !== null) {
      void followTabs(latest);
    }
  });

  // Focus that leaves the page for the app lands on the pane's controls:
  // forward on "Focus page", back on the address (ADR-0063, decision 6).
  $effect(() => {
    const returned = status?.focusReturn ?? null;
    const handled = returned === null || returned.sequence === focusSequence;
    if (handled) {
      return;
    }
    const first = focusSequence === null;
    focusSequence = returned.sequence;
    if (!first) {
      (returned.forward ? focusButton : address)?.focus();
    }
  });
</script>

<section
  class="flex h-full min-h-0 min-w-0 flex-col gap-2 overflow-hidden p-3"
  aria-label={m.browser_panel_title()}
  data-browser-conversation={target.conversationId}
  data-browser-panel=""
  data-browser-tab={target.tabId ?? undefined}
>
  {#if session.failure !== null}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.browser_error_title()}</Alert.Title>
      <Alert.Description>{session.failure}</Alert.Description>
    </Alert.Root>
  {/if}
  {#if session.loading}
    <div class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
      <Spinner aria-hidden="true" aria-label={m.browser_loading()} />
      {m.browser_loading()}
    </div>
  {:else if status === null}
    <Button class="self-start" size="sm" variant="outline" onclick={() => session.follow()}>
      {m.browser_retry()}
    </Button>
  {:else if status.control === null}
    <Empty.Root>
      <Empty.Header>
        <Empty.Media variant="icon"><GlobeIcon /></Empty.Media>
        <Empty.Title>{m.browser_empty_title()}</Empty.Title>
        <Empty.Description>{ended ?? m.browser_empty_description()}</Empty.Description>
      </Empty.Header>
      <Empty.Content>
        <Button disabled={session.busy} onclick={() => void start()}>
          {#if session.busy}<Spinner
              aria-hidden="true"
              aria-label={m.browser_loading()}
              data-icon="inline-start"
            />{/if}
          {m.browser_start()}
        </Button>
      </Empty.Content>
    </Empty.Root>
  {:else}
    <div class="flex flex-wrap items-center gap-2">
      <span class="me-auto text-sm text-muted-foreground" role="status">
        {owner ? m.browser_owner_control() : m.browser_agent_control()}
      </span>
      {#if owner}
        <Button disabled={session.busy} size="sm" onclick={() => void control({ kind: 'resume' })}>
          {m.browser_resume()}
        </Button>
      {:else}
        <Button disabled={session.busy} size="sm" onclick={() => void control({ kind: 'take_over' })}>
          {m.browser_take_over()}
        </Button>
      {/if}
      <Button disabled={session.busy} size="sm" variant="outline" onclick={() => (confirmingEnd = true)}>
        {m.browser_end()}
      </Button>
      <BrowserCredentials
        busy={session.busy}
        credentials={status.credentials}
        failure={session.failure}
        save={saveCredential}
      />
      {#if owner}
        <Button
          aria-label={m.browser_new_tab()}
          disabled={session.busy}
          size="icon-sm"
          title={m.browser_new_tab()}
          variant="ghost"
          onclick={() => void openTab()}
        >
          <PlusIcon />
        </Button>
        {#if tab !== undefined}
          <Button
            aria-label={m.browser_close_tab()}
            disabled={session.busy}
            size="icon-sm"
            title={m.browser_close_tab()}
            variant="ghost"
            onclick={() => void closeTab(tab.id)}
          >
            <XIcon />
          </Button>
        {/if}
      {/if}
    </div>
    {#if tab === undefined}
      {#if activeTab === undefined}
        <p class="text-sm text-muted-foreground">{m.browser_no_tab()}</p>
      {:else}
        <Button class="self-start" size="sm" variant="outline" onclick={() => void showActiveTab(argument)}>
          {m.browser_open_panel()}
        </Button>
      {/if}
    {:else}
      <form class="flex items-center gap-1" onsubmit={(event) => void navigate(event)}>
        <Button
          aria-label={m.browser_back()}
          disabled={!interactive || !tab.canGoBack}
          size="icon-sm"
          title={m.browser_back()}
          type="button"
          variant="ghost"
          onclick={() => void control({ kind: 'back' })}><ArrowLeftIcon /></Button
        >
        <Button
          aria-label={m.browser_forward()}
          disabled={!interactive || !tab.canGoForward}
          size="icon-sm"
          title={m.browser_forward()}
          type="button"
          variant="ghost"
          onclick={() => void control({ kind: 'forward' })}><ArrowRightIcon /></Button
        >
        <Button
          aria-label={m.browser_reload()}
          disabled={!interactive}
          size="icon-sm"
          title={m.browser_reload()}
          type="button"
          variant="ghost"
          onclick={() => void control({ kind: 'reload' })}
        >
          {#if tab.loading}<Spinner aria-hidden="true" />{:else}<RotateCwIcon />{/if}
        </Button>
        <Input
          class="min-w-0 flex-1"
          aria-label={m.browser_address()}
          autocomplete="off"
          placeholder={m.browser_address_placeholder()}
          readonly={!interactive}
          spellcheck={false}
          value={draftAddress ?? tab.url}
          bind:ref={address}
          oninput={editAddress}
        />
        <Button disabled={!interactive} size="sm" type="submit" variant="outline">{m.browser_go()}</Button>
      </form>
      {#key tab.id}
        <BrowserViewport conversationId={target.conversationId} tabId={tab.id} />
      {/key}
      <div class="flex flex-wrap items-center gap-2">
        <Button
          data-browser-focus-page=""
          disabled={!owner}
          size="sm"
          variant="outline"
          bind:ref={focusButton}
          onclick={() => void session.focusPage(tab.id)}
        >
          {m.browser_focus_page()}
        </Button>
        <span class="text-xs text-muted-foreground">{m.browser_exit_keyboard()}</span>
        <span class="ms-auto text-xs text-muted-foreground" aria-live="polite" data-browser-activity="" role="status">
          {activity}
        </span>
      </div>
    {/if}
  {/if}
</section>

<AlertDialog.Root bind:open={confirmingEnd}>
  <AlertDialog.Content>
    <AlertDialog.Header>
      <AlertDialog.Title>{m.browser_end_question()}</AlertDialog.Title>
      <AlertDialog.Description>{m.browser_end_description()}</AlertDialog.Description>
    </AlertDialog.Header>
    <AlertDialog.Footer>
      <AlertDialog.Cancel>{m.browser_cancel()}</AlertDialog.Cancel>
      <AlertDialog.Action onclick={() => void end()}>{m.browser_end()}</AlertDialog.Action>
    </AlertDialog.Footer>
  </AlertDialog.Content>
</AlertDialog.Root>
