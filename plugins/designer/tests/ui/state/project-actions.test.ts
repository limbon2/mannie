import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ProjectView } from '../../../ui/generated/bindings.js';
import { type NameRequest, ProjectActions } from '../../../ui/state/project-actions.svelte.js';
import { projectSelection } from '../../../ui/state/project-selection.svelte.js';
import { projectLibrary } from '../../../ui/state/projects.svelte.js';

const core = vi.hoisted(() => ({ createProject: vi.fn() }));

vi.mock('../../../ui/generated/bindings.js', () => ({ commands: { createProject: core.createProject } }));

function project(id: string, name: string): ProjectView {
  return { createdAt: 0, id, name };
}

/** The value, which the test expects to be there. */
function present<T>(value: T | null): T {
  expect(value).not.toBeNull();
  return value as T;
}

beforeEach(() => {
  core.createProject.mockReset();
  projectLibrary.projects = [];
  projectSelection.id = null;
});

describe('the dialog that names a project', () => {
  it('opens with an empty name', () => {
    const actions = new ProjectActions();

    actions.startCreate();

    expect(actions.nameOpen).toBe(true);
    expect(present(actions.name).name).toBe('');
  });

  it('creates and selects the project that the owner named', async () => {
    const created = project('project-1', 'Landing page');
    core.createProject.mockResolvedValue({ data: created, status: 'ok' });
    const actions = new ProjectActions();

    actions.startCreate();
    const request: NameRequest = present(actions.name);

    request.name = 'Landing page';

    await actions.confirmName(request);

    expect(core.createProject).toHaveBeenCalledWith({ name: 'Landing page' });
    expect(projectLibrary.projects).toEqual([created]);
    expect(projectSelection.id).toBe(created.id);
    expect(projectSelection.shown()).toEqual(created);
    expect(actions.nameOpen).toBe(false);
  });

  it('keeps the dialog open with the reason when the plugin refuses the name', async () => {
    const error = { code: 'name_taken', description: '', origin: 'plugin' };
    core.createProject.mockResolvedValue({ error, status: 'error' });
    const actions = new ProjectActions();

    actions.startCreate();
    const request: NameRequest = present(actions.name);

    request.name = 'Landing page';

    await actions.confirmName(request);

    expect(actions.nameOpen).toBe(true);
    expect(request.failure).toEqual({ error, kind: 'core' });
    expect(request.saving).toBe(false);
    expect(projectLibrary.projects).toEqual([]);
  });
});
