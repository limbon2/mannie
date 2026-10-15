// How the sidebar groups conversations (docs/features/chat/projects.md, the
// sidebar): each project with its conversations, the most recently active
// project first, and the general conversations apart. A conversation whose
// project the list does not know yet stays with the general ones until the
// project arrives.
import type { ConversationSummary, ProjectView } from '#lib/generated/bindings.js';

/** How many conversations a project shows before "Show more". */
export const PROJECT_CONVERSATIONS_SHOWN = 5;

/** A project with its conversations, as the sidebar lists it. */
export interface ProjectGroup {
  /** When the project was last active: its newest conversation's change, or its creation while it has none. */
  readonly activeAt: number;
  /** Whether one of its conversations waits for the owner's approval. */
  readonly awaitingApproval: boolean;
  /** Its conversations, the most recent first. */
  readonly conversations: readonly ConversationSummary[];
  readonly project: ProjectView;
  /** Whether one of its conversations writes a reply. */
  readonly replying: boolean;
}

/** The sidebar's conversations, by project. */
export interface GroupedConversations {
  /** The conversations outside every project the list knows, the most recent first. */
  readonly general: readonly ConversationSummary[];
  /** The projects, the most recently active first. */
  readonly projects: readonly ProjectGroup[];
}

function byActivity(left: ProjectGroup, right: ProjectGroup): number {
  const byTime = right.activeAt - left.activeAt;
  if (byTime !== 0) {
    return byTime;
  }
  return right.project.id.localeCompare(left.project.id);
}

/**
 * Groups conversations by their project. `conversations` come the most
 * recent first, as the list keeps them, and keep their order in each group.
 */
export function groupConversations(
  conversations: readonly ConversationSummary[],
  projects: readonly ProjectView[],
): GroupedConversations {
  const byProject = new Map<string, ConversationSummary[]>(projects.map((project) => [project.id, []]));
  const general: ConversationSummary[] = [];
  for (const conversation of conversations) {
    const projectId = conversation.projectId;
    const members = projectId === null ? undefined : byProject.get(projectId);
    if (members === undefined) {
      general.push(conversation);
    } else {
      members.push(conversation);
    }
  }
  const groups = projects.map((project): ProjectGroup => {
    const members = byProject.get(project.id) ?? [];
    return {
      activeAt: members[0]?.updatedAt ?? project.createdAt,
      awaitingApproval: members.some((conversation) => conversation.awaitingApproval),
      conversations: members,
      project,
      replying: members.some((conversation) => conversation.hasActiveRun),
    };
  });
  return { general, projects: groups.sort(byActivity) };
}

/**
 * The conversations a project shows: all of them after "Show more", or the
 * first few and the open one, so the open conversation is always in view.
 */
export function shownConversations(
  conversations: readonly ConversationSummary[],
  showAll: boolean,
  openId: string | null,
): readonly ConversationSummary[] {
  if (showAll) {
    return conversations;
  }
  const first = conversations.slice(0, PROJECT_CONVERSATIONS_SHOWN);
  const openIndex = conversations.findIndex((conversation) => conversation.id === openId);
  const isOpenHidden = openIndex >= PROJECT_CONVERSATIONS_SHOWN;
  return isOpenHidden ? [...first, conversations[openIndex]] : first;
}
