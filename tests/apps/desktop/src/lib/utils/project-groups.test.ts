// How the sidebar groups conversations by project (docs/features/chat/
// projects.md, the sidebar).
import { describe, expect, it } from 'vitest';

import type { ConversationSummary, ProjectView } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import {
  groupConversations,
  PROJECT_CONVERSATIONS_SHOWN,
  shownConversations,
} from '../../../../../../apps/desktop/src/lib/utils/project-groups.js';

function project(id: string, createdAt: number): ProjectView {
  return { createdAt, folder: `D:\\Projects\\${id}`, id, name: id };
}

function conversation(id: string, projectId: string | null, updatedAt: number): ConversationSummary {
  return {
    awaitingApproval: false,
    createdAt: updatedAt,
    hasActiveRun: false,
    id,
    profileId: null,
    projectId,
    task: null,
    title: id,
    titleSource: 'first_message',
    updatedAt,
    workspace: null,
  };
}

describe('groupConversations', () => {
  it('lists each project with its conversations and keeps the general ones apart', () => {
    const conversations = [
      conversation('newest', 'app', 50),
      conversation('general', null, 40),
      conversation('older', 'app', 30),
    ];

    const grouped = groupConversations(conversations, [project('app', 1)]);

    expect(grouped.general.map((item) => item.id)).toEqual(['general']);
    expect(grouped.projects.map((group) => group.project.id)).toEqual(['app']);
    expect(grouped.projects[0]?.conversations.map((item) => item.id)).toEqual(['newest', 'older']);
    expect(grouped.projects[0]?.activeAt).toBe(50);
  });

  it('puts the most recently active project first, and an empty one by when it was added', () => {
    const projects = [project('quiet', 10), project('busy', 1), project('new', 90)];
    const conversations = [conversation('a', 'busy', 80), conversation('b', 'quiet', 20)];

    const grouped = groupConversations(conversations, projects);

    expect(grouped.projects.map((group) => group.project.id)).toEqual(['new', 'busy', 'quiet']);
    expect(grouped.projects[0]?.conversations).toEqual([]);
  });

  it('keeps a conversation of a project that the list does not know yet with the general ones', () => {
    const grouped = groupConversations([conversation('moved', 'unknown', 5)], []);

    expect(grouped.general.map((item) => item.id)).toEqual(['moved']);
    expect(grouped.projects).toEqual([]);
  });

  it('says when a conversation of a project waits for approval or writes a reply', () => {
    const waiting = { ...conversation('waiting', 'app', 5), awaitingApproval: true };
    const writing = { ...conversation('writing', 'other', 4), hasActiveRun: true };

    const grouped = groupConversations([waiting, writing], [project('app', 1), project('other', 2)]);

    const byId = new Map(grouped.projects.map((group) => [group.project.id, group]));
    expect(byId.get('app')?.awaitingApproval).toBe(true);
    expect(byId.get('app')?.replying).toBe(false);
    expect(byId.get('other')?.replying).toBe(true);
  });
});

describe('shownConversations', () => {
  const many = Array.from({ length: 8 }, (_, index) => conversation(`c${String(index)}`, 'app', 100 - index));

  it('shows the first few until the owner asks for more', () => {
    expect(shownConversations(many, false, null)).toHaveLength(PROJECT_CONVERSATIONS_SHOWN);
    expect(shownConversations(many, true, null)).toHaveLength(8);
  });

  it('keeps the open conversation in view even beyond the first few', () => {
    const shown = shownConversations(many, false, 'c7');

    expect(shown.map((item) => item.id)).toEqual(['c0', 'c1', 'c2', 'c3', 'c4', 'c7']);
  });
});
