<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as DropdownMenu from '@mannie/plugin-sdk/components/dropdown-menu';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import AtSignIcon from '@mannie/plugin-sdk/icons/at-sign';
  import CheckIcon from '@mannie/plugin-sdk/icons/check';
  import Columns2Icon from '@mannie/plugin-sdk/icons/columns-2';
  import CopyIcon from '@mannie/plugin-sdk/icons/copy';
  import EllipsisVerticalIcon from '@mannie/plugin-sdk/icons/ellipsis-vertical';
  import GitCompareIcon from '@mannie/plugin-sdk/icons/git-compare';
  import Rows2Icon from '@mannie/plugin-sdk/icons/rows-2';
  import XIcon from '@mannie/plugin-sdk/icons/x';

  import { m } from '../../generated/messages.js';
  import type { RepositoryPanel } from '../../state/git-repository.svelte.js';
  import { formatFullDate, formatRelativeTime, mentionEntryForCommit } from '../../utils/git-format.js';
  import { gitFailureMessage } from '../../utils/git-messages.js';
  import DiffView from '../cards/DiffView.svelte';
  import RefBadge from '../cards/RefBadge.svelte';

  // Split-pane commit inspector: commit metadata, clickable parents,
  // copy SHA, Mention in chat, Compare with HEAD, branch/tag/cherry-pick/
  // revert/checkout actions, and full file diffs.
  interface Props {
    readonly onClose: () => void;
    readonly panel: RepositoryPanel;
  }

  let { onClose, panel }: Props = $props();

  let copiedSha = $state(false);

  const view = $derived(panel.commit.value);
  const isLoadingInitial = $derived(panel.commit.status === 'loading' && view === null);
  const loadFailure = $derived(panel.commit.status === 'error' ? panel.commit.failure : null);
  const hasSeparateCommitter = $derived.by(() => {
    if (view === null) {
      return false;
    }
    const isDifferentPerson = view.commit.committer !== view.commit.author;
    const isDifferentTime = view.commit.committed !== view.commit.authored;
    return isDifferentPerson || isDifferentTime;
  });
  const subject = $derived(view?.commit.message.split('\n')[0] ?? '');
  const body = $derived.by(() => {
    const raw = view?.commit.message ?? '';
    const firstBreak = raw.indexOf('\n');
    return firstBreak === -1 ? '' : raw.slice(firstBreak + 1).trim();
  });

  function copySha(): void {
    const oid = view?.commit.oid;
    if (oid === undefined) {
      return;
    }
    void navigator.clipboard.writeText(oid).then(() => {
      copiedSha = true;
      setTimeout(() => {
        copiedSha = false;
      }, 1500);
    });
  }

  function mentionCommit(): void {
    if (view === null) {
      return;
    }
    panel.mentionInChat(mentionEntryForCommit({ oid: view.commit.oid, short: view.commit.short, subject }));
  }

  function compareWithHead(): void {
    if (view === null) {
      return;
    }
    panel.openCompare(view.commit.oid, 'HEAD');
  }

  function toggleDiffMode(): void {
    panel.diffMode = panel.diffMode === 'unified' ? 'split' : 'unified';
  }

  function createBranchHere(): void {
    if (view !== null) {
      panel.openDialog({ kind: 'create_branch', start: view.commit.oid });
    }
  }

  function createTagHere(): void {
    if (view !== null) {
      panel.openDialog({ kind: 'create_tag', target: view.commit.oid });
    }
  }

  function cherryPickCommit(): void {
    if (view !== null) {
      void panel.runAction({ commit: view.commit.oid, kind: 'cherry_pick', noCommit: false });
    }
  }

  function revertCommit(): void {
    if (view !== null) {
      void panel.runAction({ commit: view.commit.oid, kind: 'revert', noCommit: false });
    }
  }

  function checkoutDetached(): void {
    if (view !== null) {
      void panel.runAction({
        create: false,
        detach: true,
        kind: 'switch_branch',
        start: null,
        target: view.commit.oid,
      });
    }
  }
</script>

<div class="flex h-full min-h-0 flex-col overflow-hidden" data-git-commit-inspector="">
  {#if isLoadingInitial}
    <div class="flex items-center gap-2 p-3 text-xs text-muted-foreground" aria-busy="true">
      <Spinner aria-label={m.git_panel_loading()} />
      <span>{m.git_panel_loading()}</span>
    </div>
  {:else if loadFailure !== null}
    <div class="p-3">
      <Alert.Root variant="destructive">
        <Alert.Description class="flex items-center justify-between gap-2 text-xs">
          <span>{gitFailureMessage(loadFailure)}</span>
          <Button size="sm" variant="outline" onclick={() => panel.readCommit()}>{m.git_panel_try_again()}</Button>
        </Alert.Description>
      </Alert.Root>
    </div>
  {:else if view !== null}
    <div class="flex items-center gap-1.5 border-b bg-muted/30 px-2.5 py-1.5">
      <span class="font-mono text-xs font-semibold">{view.commit.short}</span>

      <Button
        class="size-6"
        aria-label={copiedSha ? m.git_diff_copied() : m.git_commit_copy_sha()}
        size="icon"
        title={copiedSha ? m.git_diff_copied() : m.git_commit_copy_sha()}
        variant="ghost"
        onclick={copySha}
      >
        {#if copiedSha}
          <CheckIcon class="size-3.5 text-git-added" aria-hidden="true" />
        {:else}
          <CopyIcon class="size-3.5" aria-hidden="true" />
        {/if}
      </Button>

      <div class="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
        {#each view.commit.refs as reference (reference)}
          <RefBadge {reference} />
        {/each}
      </div>

      <div class="flex shrink-0 items-center gap-0.5">
        <Button
          class="size-6"
          aria-label={m.git_panel_mention_commit()}
          size="icon"
          title={m.git_panel_mention_commit()}
          variant="ghost"
          onclick={mentionCommit}
        >
          <AtSignIcon class="size-3.5" aria-hidden="true" />
        </Button>

        <Button
          class="size-6"
          aria-label={m.git_commit_compare_head()}
          size="icon"
          title={m.git_commit_compare_head()}
          variant="ghost"
          onclick={compareWithHead}
        >
          <GitCompareIcon class="size-3.5" aria-hidden="true" />
        </Button>

        <Button
          class="size-6"
          aria-label={panel.diffMode === 'unified' ? m.git_diff_split() : m.git_diff_unified()}
          aria-pressed={panel.diffMode === 'split'}
          size="icon"
          title={panel.diffMode === 'unified' ? m.git_diff_split() : m.git_diff_unified()}
          variant={panel.diffMode === 'split' ? 'secondary' : 'ghost'}
          onclick={toggleDiffMode}
        >
          {#if panel.diffMode === 'unified'}
            <Columns2Icon class="size-3.5" aria-hidden="true" />
          {:else}
            <Rows2Icon class="size-3.5" aria-hidden="true" />
          {/if}
        </Button>

        <DropdownMenu.Root>
          <DropdownMenu.Trigger>
            {#snippet child({ props })}
              <Button
                {...props}
                class="size-6"
                aria-label={m.git_commit_actions()}
                disabled={panel.runningAction}
                size="icon"
                title={m.git_commit_actions()}
                variant="ghost"
              >
                <EllipsisVerticalIcon class="size-3.5" aria-hidden="true" />
              </Button>
            {/snippet}
          </DropdownMenu.Trigger>
          <DropdownMenu.Content align="end">
            <DropdownMenu.Item onclick={createBranchHere}>{m.git_commit_create_branch()}</DropdownMenu.Item>
            <DropdownMenu.Item onclick={createTagHere}>{m.git_commit_create_tag()}</DropdownMenu.Item>
            <DropdownMenu.Separator />
            <DropdownMenu.Item onclick={cherryPickCommit}>{m.git_commit_cherry_pick()}</DropdownMenu.Item>
            <DropdownMenu.Item onclick={revertCommit}>{m.git_commit_revert()}</DropdownMenu.Item>
            <DropdownMenu.Item onclick={checkoutDetached}>{m.git_commit_checkout_detached()}</DropdownMenu.Item>
          </DropdownMenu.Content>
        </DropdownMenu.Root>

        <Button
          class="size-6"
          aria-label={m.git_dialog_close()}
          size="icon"
          title={m.git_dialog_close()}
          variant="ghost"
          onclick={onClose}
        >
          <XIcon class="size-3.5" aria-hidden="true" />
        </Button>
      </div>
    </div>

    <div class="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3 text-xs">
      <div class="flex flex-col gap-1">
        <h3 class="text-sm font-semibold">{subject}</h3>
        {#if body.length > 0}
          <pre class="font-sans text-xs whitespace-pre-wrap text-muted-foreground">{body}</pre>
        {/if}
      </div>

      <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
        <dt class="text-muted-foreground">{m.git_commit_author()}</dt>
        <dd class="min-w-0 truncate">
          <span>{view.commit.author}</span>
          {#if view.commit.email.length > 0}
            <span class="text-muted-foreground"> &lt;{view.commit.email}&gt;</span>
          {/if}
          {#if view.commit.authored.length > 0}
            <span class="ms-1.5 text-muted-foreground" title={formatFullDate(view.commit.authored)}>
              {formatRelativeTime(view.commit.authored)}
            </span>
          {/if}
        </dd>

        {#if hasSeparateCommitter}
          <dt class="text-muted-foreground">{m.git_commit_committer()}</dt>
          <dd class="min-w-0 truncate">
            <span>{view.commit.committer}</span>
            {#if view.commit.committed.length > 0}
              <span class="ms-1.5 text-muted-foreground" title={formatFullDate(view.commit.committed)}>
                {formatRelativeTime(view.commit.committed)}
              </span>
            {/if}
          </dd>
        {/if}

        <dt class="text-muted-foreground">{m.git_commit_parents()}</dt>
        <dd class="flex flex-wrap items-center gap-1">
          {#if view.commit.parents.length === 0}
            <span class="text-muted-foreground">{m.git_commit_root()}</span>
          {:else}
            {#each view.commit.parents as parentOid, idx (parentOid)}
              <button
                class="cursor-pointer rounded-xs hover:opacity-80"
                type="button"
                onclick={() => panel.chooseCommit(parentOid)}
              >
                <Badge class="font-mono" variant="outline"
                  >{view.commit.shortParents[idx] ?? parentOid.slice(0, 7)}</Badge
                >
              </button>
            {/each}
          {/if}
        </dd>
      </dl>

      <div class="flex flex-col gap-1.5">
        <h4 class="text-xs font-medium text-muted-foreground">
          {m.git_commit_files_heading({ count: formatCount(view.files.length) })}
        </h4>
        <DiffView
          files={view.files}
          mode={panel.diffMode}
          open={view.files.length === 1}
          patches={view.patches}
          onOpenBlame={(path: string) => panel.openFileBlame(path, view.commit.oid)}
          onOpenHistory={(path: string) => panel.openFileHistory(path)}
        />
      </div>
    </div>
  {/if}
</div>
