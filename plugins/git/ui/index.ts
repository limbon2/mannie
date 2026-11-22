// The git plugin's UI (plugins/git/docs/README.md; ADR-0038, ADR-0039,
// ADR-0078): the cards of its tools' calls, the interactive Git side panel
// with its toolbar control and command palette entry, and the kind of
// mentions that names uncommitted changes, commits or branches after "@".
import { definePlugin } from '@mannie/plugin-sdk';
import { listConversations } from '@mannie/plugin-sdk/conversations';
import ArchiveIcon from '@mannie/plugin-sdk/icons/archive';
import CombineIcon from '@mannie/plugin-sdk/icons/combine';
import DownloadIcon from '@mannie/plugin-sdk/icons/download';
import FileCheckIcon from '@mannie/plugin-sdk/icons/file-check';
import FileDiffIcon from '@mannie/plugin-sdk/icons/file-diff';
import FileSearchIcon from '@mannie/plugin-sdk/icons/file-search';
import GitBranchIcon from '@mannie/plugin-sdk/icons/git-branch';
import GitCommitHorizontalIcon from '@mannie/plugin-sdk/icons/git-commit-horizontal';
import GitCompareIcon from '@mannie/plugin-sdk/icons/git-compare';
import GitMergeIcon from '@mannie/plugin-sdk/icons/git-merge';
import GitPullRequestArrowIcon from '@mannie/plugin-sdk/icons/git-pull-request-arrow';
import ListChecksIcon from '@mannie/plugin-sdk/icons/list-checks';
import ListTreeIcon from '@mannie/plugin-sdk/icons/list-tree';
import PencilLineIcon from '@mannie/plugin-sdk/icons/pencil-line';
import RotateCcwIcon from '@mannie/plugin-sdk/icons/rotate-ccw';
import ScanSearchIcon from '@mannie/plugin-sdk/icons/scan-search';
import SearchIcon from '@mannie/plugin-sdk/icons/search';
import TriangleAlertIcon from '@mannie/plugin-sdk/icons/triangle-alert';
import Undo2Icon from '@mannie/plugin-sdk/icons/undo-2';
import UploadIcon from '@mannie/plugin-sdk/icons/upload';
import { openConversationId, openSidePanel } from '@mannie/plugin-sdk/navigation';
import type { ToolKindDetailsProps } from '@mannie/plugin-sdk/slots';
import type { Component } from 'svelte';

import BlameCard from './components/cards/BlameCard.svelte';
import ConflictsCard from './components/cards/ConflictsCard.svelte';
import DiffCard from './components/cards/DiffCard.svelte';
import GrepCard from './components/cards/GrepCard.svelte';
import LogCard from './components/cards/LogCard.svelte';
import OutcomeCard from './components/cards/OutcomeCard.svelte';
import RefsCard from './components/cards/RefsCard.svelte';
import StatusCard from './components/cards/StatusCard.svelte';
import TransferCard from './components/cards/TransferCard.svelte';
import GitControl from './components/GitControl.svelte';
import GitPanel from './components/GitPanel.svelte';
import { m } from './generated/messages.js';
import { gitMentionRefusal, gitMentionSummary, gitMentionVariantName, gitToolFailure } from './utils/git-messages.js';
import { GIT_PANEL } from './utils/git-panel.js';
import { SUMMARIES } from './utils/git-summaries.js';

/** A git tool's card: its icon, and what its data shows below the call's accesses. */
interface ToolCard {
  readonly details?: Component<ToolKindDetailsProps>;
  readonly icon: Component;
}

/** The card of each of the plugin's tools, by the tool's name. */
const CARDS: Readonly<Record<string, ToolCard>> = {
  git_blame: { details: BlameCard, icon: ScanSearchIcon },
  git_branch: { icon: GitBranchIcon },
  git_cherry_pick: { details: OutcomeCard, icon: CombineIcon },
  git_commit: { details: DiffCard, icon: GitCommitHorizontalIcon },
  git_conflicts: { details: ConflictsCard, icon: TriangleAlertIcon },
  git_diff: { details: DiffCard, icon: FileDiffIcon },
  git_edit_history: { icon: ListTreeIcon },
  git_fetch: { details: TransferCard, icon: DownloadIcon },
  git_grep: { details: GrepCard, icon: SearchIcon },
  git_log: { details: LogCard, icon: GitCommitHorizontalIcon },
  git_merge: { details: OutcomeCard, icon: GitMergeIcon },
  git_operation: { details: OutcomeCard, icon: GitPullRequestArrowIcon },
  git_push: { details: TransferCard, icon: UploadIcon },
  git_rebase: { details: OutcomeCard, icon: GitPullRequestArrowIcon },
  git_refs: { details: RefsCard, icon: GitBranchIcon },
  git_reset: { icon: RotateCcwIcon },
  git_resolve: { icon: PencilLineIcon },
  git_restore: { icon: Undo2Icon },
  git_revert: { details: OutcomeCard, icon: Undo2Icon },
  git_show: { details: DiffCard, icon: FileSearchIcon },
  git_stage: { icon: FileCheckIcon },
  git_stash: { details: OutcomeCard, icon: ArchiveIcon },
  git_status: { details: StatusCard, icon: ListChecksIcon },
  git_switch: { icon: GitCompareIcon },
};

/** The icon of an entry of the "@" list, by its variant. */
function mentionIcon(variant: string): Component {
  switch (variant) {
    case 'branch': {
      return GitBranchIcon;
    }
    case 'commit': {
      return GitCommitHorizontalIcon;
    }
    default: {
      return FileDiffIcon;
    }
  }
}

/** The project ID of the currently open conversation, or `null`. */
function openProjectId(): string | null {
  const conversationId = openConversationId();
  if (conversationId === null) {
    return null;
  }
  const found = listConversations().find((item) => item.id === conversationId);
  return found?.projectId ?? null;
}

export default definePlugin({
  register(context) {
    const { slots } = context;
    for (const [tool, card] of Object.entries(CARDS)) {
      slots.toolKinds.register({
        ...(card.details === undefined ? {} : { details: card.details }),
        failure: gitToolFailure,
        icon: card.icon,
        id: tool,
        summary: SUMMARIES[tool] ?? (() => null),
      });
    }
    slots.sidePanels.register({
      component: GitPanel,
      defaultLabel: m.git_panel_title,
      hideHeading: true,
      icon: GitBranchIcon,
      id: GIT_PANEL,
    });
    slots.conversationControls.register({ component: GitControl, id: 'git.control', order: 30 });
    slots.commands.register({
      available: () => openProjectId() !== null,
      group: m.git_plugin_name,
      id: 'git.open_panel',
      label: m.git_panel_open_command,
      order: 30,
      run: () => {
        const projectId = openProjectId();
        if (projectId !== null) {
          void openSidePanel(GIT_PANEL, projectId);
        }
      },
    });
    // The provider's ID is the plugin's.
    slots.mentionKinds.register({
      group: m.git_mention_group,
      icon: mentionIcon,
      id: context.id,
      order: 30,
      refusal: gitMentionRefusal,
      summary: gitMentionSummary,
      variantName: gitMentionVariantName,
      // The provider warns about no entry.
      warning: () => null,
    });
  },
});
