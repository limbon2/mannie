<script lang="ts">
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import GitBranchIcon from '@mannie/plugin-sdk/icons/git-branch';
  import { batchChannel, call, type ChannelMessage, type SubscriptionId, unsubscribe } from '@mannie/plugin-sdk/ipc';
  import { openSidePanel } from '@mannie/plugin-sdk/navigation';
  import type { ConversationControlProps } from '@mannie/plugin-sdk/slots';

  import { channels, commands, type RepositoryOverview, type RepositoryUpdate } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { GIT_PANEL } from '../utils/git-panel.js';

  // The Git button in a conversation's toolbar (ADR-0026; plugins/git/docs/
  // panel.md): in a conversation whose project is a repository, it names the
  // current branch, reflects dirty/conflicted counts live via
  // `follow_repository`, and opens the Git side panel for the project.
  let { conversation, turns }: ConversationControlProps = $props();

  let overview = $state<RepositoryOverview | null>(null);
  let trigger = $state<HTMLElement | null>(null);

  const projectId = $derived(conversation.projectId);
  const turnCount = $derived(turns.length);
  const runState = $derived(turns.at(-1)?.run?.state ?? null);
  /**
   * What the branch is read for: the project, and again when a turn starts
   * and when its run ends, since agents may switch branches.
   */
  const readFor = $derived(projectId === null ? null : { projectId, runState, turnCount });
  const branch = $derived(overview?.branch.head ?? null);
  const label = $derived(branch === null ? m.git_control_detached() : m.git_control_label({ branch }));
  const conflictedCount = $derived(
    overview === null ? 0 : overview.entries.filter((entry) => entry.kind === 'conflicted').length,
  );
  const dirtyCount = $derived(overview?.entriesTotal ?? 0);

  function open(): void {
    if (projectId !== null) {
      void openSidePanel(GIT_PANEL, projectId, { opener: trigger });
    }
  }

  $effect(() => {
    const target = readFor;
    if (target === null) {
      overview = null;
      return;
    }
    let isCurrent = true;
    let subscription: SubscriptionId | null = null;

    function loadOverview(): void {
      if (target === null) {
        return;
      }
      void call(() => commands.repositoryOverview({ projectId: target.projectId })).then((outcome) => {
        if (isCurrent) {
          overview = outcome.ok ? outcome.data : null;
        }
      });
    }

    loadOverview();

    const channel = batchChannel<ChannelMessage<RepositoryUpdate>>(
      (messages) => {
        const hasItem = messages.some((message) => message.kind === 'item');
        if (isCurrent && hasItem) {
          loadOverview();
        }
      },
      () => undefined,
    );
    void call(() => channels.followRepository(channel)).then((outcome) => {
      if (!outcome.ok) {
        return;
      }
      if (isCurrent) {
        subscription = outcome.data;
      } else {
        void unsubscribe(outcome.data);
      }
    });

    return () => {
      isCurrent = false;
      if (subscription !== null) {
        void unsubscribe(subscription);
      }
    };
  });
</script>

{#if overview !== null}
  <!-- The visible branch is the button's name (WCAG 2.5.3); the title says what it does. -->
  <Button data-git-control="" size="sm" title={m.git_control_open()} variant="ghost" bind:ref={trigger} onclick={open}>
    <GitBranchIcon data-icon="inline-start" />
    <span class="max-w-40 truncate">{label}</span>
    {#if conflictedCount > 0}
      <Badge class="h-4 px-1 text-[10px]" variant="destructive">{formatCount(conflictedCount)}</Badge>
    {:else if dirtyCount > 0}
      <Badge class="h-4 px-1 text-[10px]" variant="secondary">{formatCount(dirtyCount)}</Badge>
    {/if}
  </Button>
{/if}
