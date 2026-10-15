// The owner's actions on projects (docs/features/chat/projects.md): what each
// sends to the core, where the window goes next, and what the dialogs show
// when the core refuses.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  ChatError,
  ConversationSummary,
  ProjectView,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { conversationList } from '../../../../../../apps/desktop/src/lib/state/conversation-list.svelte.js';
import { ProjectActions } from '../../../../../../apps/desktop/src/lib/state/project-actions.svelte.js';
import { sidebarCollapse } from '../../../../../../apps/desktop/src/lib/state/sidebar-collapse.svelte.js';

const core = vi.hoisted(() => ({
  addProject: vi.fn(),
  addProjectFromPath: vi.fn(),
  removeProject: vi.fn(),
  renameProject: vi.fn(),
}));

const navigation = vi.hoisted(() => ({ goto: vi.fn() }));

const page = vi.hoisted(() => ({ route: { id: '/' }, url: new URL('http://localhost/') }));

vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: core }));
vi.mock('$app/navigation', () => navigation);
vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));
vi.mock('$app/state', () => ({ page }));

const PROJECT: ProjectView = {
  createdAt: 1,
  folder: 'D:\\Projects\\mannie',
  id: '0199e0f2-0000-7000-a000-000000000001',
  name: 'mannie',
};

const CONVERSATION: ConversationSummary = {
  awaitingApproval: false,
  createdAt: 2,
  hasActiveRun: false,
  id: '0199e0f2-0000-7000-8000-000000000001',
  profileId: null,
  projectId: PROJECT.id,
  title: 'In mannie',
  titleSource: 'first_message',
  updatedAt: 2,
  workspace: PROJECT.folder,
};

function ok<T>(data: T): { data: T; status: 'ok' } {
  return { data, status: 'ok' };
}

function refused(error: ChatError): { error: ChatError & { description: string }; status: 'error' } {
  return { error: { ...error, description: 'refused' }, status: 'error' };
}

beforeEach(() => {
  for (const mock of [...Object.values(core), navigation.goto]) {
    mock.mockReset();
  }
  page.url = new URL('http://localhost/');
  conversationList.conversations = [];
  conversationList.projects = [];
});

describe('adding a project', () => {
  it('opens a new conversation in the project that the folder dialog added', async () => {
    const actions = new ProjectActions();
    sidebarCollapse.setProjectOpen(PROJECT.id, false);
    core.addProject.mockResolvedValue(ok(PROJECT));

    await actions.add();

    expect(conversationList.findProject(PROJECT.id)).toEqual(PROJECT);
    expect(sidebarCollapse.isProjectOpen(PROJECT.id)).toBe(true);
    expect(navigation.goto).toHaveBeenCalledWith(`/?project=${PROJECT.id}&view=chat`);
  });

  it('changes nothing when the owner closes the folder dialog', async () => {
    const actions = new ProjectActions();
    core.addProject.mockResolvedValue(ok(null));

    await actions.add();

    expect(navigation.goto).not.toHaveBeenCalled();
    expect(actions.addPathOpen).toBe(false);
  });

  it('offers to type the folder, with the cause, when the dialog failed or the folder was refused', async () => {
    const actions = new ProjectActions();
    core.addProject.mockResolvedValue(refused({ code: 'folder_dialog_failed' }));

    await actions.add();

    expect(actions.addPathOpen).toBe(true);
    expect(actions.addPath?.failure).toEqual({
      error: { code: 'folder_dialog_failed', description: 'refused' },
      kind: 'core',
    });
  });

  it('keeps the typed folder’s dialog open with the cause until the core accepts a folder', async () => {
    const actions = new ProjectActions();
    actions.startAddFromPath();
    const request = actions.addPath;
    if (request === null) {
      throw new Error('the dialog did not open');
    }
    request.folder = 'D:\\Missing';

    core.addProjectFromPath.mockResolvedValueOnce(refused({ code: 'folder_not_found' }));
    await actions.saveAddFromPath(request);
    expect(actions.addPathOpen).toBe(true);
    expect(request.failure?.kind).toBe('core');

    request.folder = PROJECT.folder;
    core.addProjectFromPath.mockResolvedValueOnce(ok(PROJECT));
    await actions.saveAddFromPath(request);
    expect(core.addProjectFromPath).toHaveBeenLastCalledWith({ folder: PROJECT.folder });
    expect(actions.addPathOpen).toBe(false);
    expect(navigation.goto).toHaveBeenCalledWith(`/?project=${PROJECT.id}&view=chat`);
  });
});

describe('renaming a project', () => {
  it('keeps the dialog open for a refused name, and closes once the core accepted one', async () => {
    const actions = new ProjectActions();
    conversationList.projects = [PROJECT];
    actions.startRename(PROJECT);
    const request = actions.renaming;
    if (request === null) {
      throw new Error('the dialog did not open');
    }
    expect(request.name).toBe('mannie');

    request.name = ' ';
    core.renameProject.mockResolvedValueOnce(refused({ code: 'project_name_invalid', maxCharacters: 100 }));
    await actions.saveRename(request);
    expect(actions.renameOpen).toBe(true);

    request.name = 'Mannie app';
    core.renameProject.mockResolvedValueOnce(ok({ ...PROJECT, name: 'Mannie app' }));
    await actions.saveRename(request);
    expect(core.renameProject).toHaveBeenLastCalledWith({ name: 'Mannie app', projectId: PROJECT.id });
    expect(actions.renameOpen).toBe(false);
    expect(conversationList.findProject(PROJECT.id)?.name).toBe('Mannie app');
  });
});

describe('removing a project', () => {
  it('asks again with the new number when the project changed meanwhile', async () => {
    const actions = new ProjectActions();
    actions.startRemove(PROJECT, 1);
    const request = actions.removing;
    if (request === null) {
      throw new Error('the dialog did not open');
    }
    core.removeProject.mockResolvedValueOnce(refused({ code: 'project_changed', conversations: 2 }));

    await actions.confirmRemove(request);

    expect(core.removeProject).toHaveBeenCalledWith({ conversations: 1, projectId: PROJECT.id });
    expect(actions.removeOpen).toBe(true);
    expect(request.conversations).toBe(2);
    expect(request.failure?.kind).toBe('core');
  });

  it('closes, and leaves an open conversation of the project for a new one', async () => {
    const actions = new ProjectActions();
    conversationList.conversations = [CONVERSATION];
    page.url = new URL(`http://localhost/?conversation=${CONVERSATION.id}&view=chat`);
    actions.startRemove(PROJECT, 1);
    const request = actions.removing;
    if (request === null) {
      throw new Error('the dialog did not open');
    }
    core.removeProject.mockResolvedValueOnce(ok(null));

    await actions.confirmRemove(request);

    expect(actions.removeOpen).toBe(false);
    expect(navigation.goto).toHaveBeenCalledWith('/?view=chat');
  });

  it('stays where the owner is when the removed project is not shown', async () => {
    const actions = new ProjectActions();
    page.url = new URL('http://localhost/?view=chat');
    actions.startRemove(PROJECT, 0);
    const request = actions.removing;
    if (request === null) {
      throw new Error('the dialog did not open');
    }
    core.removeProject.mockResolvedValueOnce(ok(null));

    await actions.confirmRemove(request);

    expect(navigation.goto).not.toHaveBeenCalled();
  });
});
