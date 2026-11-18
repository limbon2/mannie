// The projects that the window follows: they are ready once the channel
// brought the first list, each list replaces the one before, a list that fell
// behind follows again, a failure can be tried again, a command's project shows
// until the channel brings it, and the list forgets the projects as the plugin
// turns off.
import type * as Ipc from '@mannie/plugin-sdk/ipc';
import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ProjectList, ProjectView } from '../../../ui/generated/bindings.js';
import { ProjectLibrary } from '../../../ui/state/projects.svelte.js';

const core = vi.hoisted(() => ({ followProjects: vi.fn(), unsubscribe: vi.fn() }));

type ProjectMessages = readonly Ipc.ChannelMessage<ProjectList>[];

const channel = vi.hoisted(() => ({ deliver: vi.fn<(messages: ProjectMessages) => void>() }));

vi.mock('../../../ui/generated/bindings.js', () => ({ channels: { followProjects: core.followProjects } }));
vi.mock('@mannie/plugin-sdk/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  batchChannel: (onItems: (messages: ProjectMessages) => void) => {
    channel.deliver.mockImplementation(onItems);
    return {};
  },
  unsubscribe: core.unsubscribe,
}));

function project(id: string): ProjectView {
  return { createdAt: 0, id, name: `Project ${id}` };
}

interface FollowedLibrary {
  context: TestPluginContext;
  library: ProjectLibrary;
}

/** Follows the projects in a context where the plugin is on. */
async function followedLibrary(): Promise<FollowedLibrary> {
  const context = new TestPluginContext('designer', true);
  const library = new ProjectLibrary();
  library.follow(context);
  await vi.waitFor(() => {
    expect(core.followProjects).toHaveBeenCalled();
  });
  return { context, library };
}

beforeEach(() => {
  channel.deliver.mockReset();
  for (const command of Object.values(core)) {
    command.mockReset();
  }
  core.followProjects.mockResolvedValue({ data: 7, status: 'ok' });
});

describe('the projects', () => {
  it('are ready once the first list arrives, and each list replaces the one before', async () => {
    const { library } = await followedLibrary();
    await Promise.resolve();
    const beforeTheList = library.status;

    channel.deliver([{ item: { projects: [project('project-1')] }, kind: 'item' }]);
    const first = library.projects.map((known) => known.id);
    channel.deliver([{ item: { projects: [project('project-2'), project('project-1')] }, kind: 'item' }]);

    expect(beforeTheList).toBe('loading');
    expect(first).toEqual(['project-1']);
    expect(library.status).toBe('ready');
    expect(library.projects.map((known) => known.id)).toEqual(['project-2', 'project-1']);
  });

  it('follow again when the list fell behind', async () => {
    await followedLibrary();

    channel.deliver([{ kind: 'ended', reason: 'lagged' }]);

    await vi.waitFor(() => {
      expect(core.followProjects).toHaveBeenCalledTimes(2);
    });
  });

  it('records a subscription failure and recovers when retried', async () => {
    const error = { code: 'storage_failed', description: '', origin: 'plugin' };
    core.followProjects.mockResolvedValueOnce({ error, status: 'error' });
    const { library } = await followedLibrary();

    await vi.waitFor(() => {
      expect(library.status).toBe('error');
    });
    expect(library.failure).toEqual({ error, kind: 'core' });

    library.retry();

    expect(library.status).toBe('loading');
    await vi.waitFor(() => {
      expect(core.followProjects).toHaveBeenCalledTimes(2);
      expect(library.failure).toBeNull();
    });

    const loaded = project('project-1');
    channel.deliver([{ item: { projects: [loaded] }, kind: 'item' }]);

    expect(library.status).toBe('ready');
    expect(library.projects).toEqual([loaded]);
  });

  it('keep a project that a command returned until the channel brings it', async () => {
    const { library } = await followedLibrary();
    channel.deliver([{ item: { projects: [project('project-1')] }, kind: 'item' }]);

    library.keep(project('project-2'));
    library.keep(project('project-2'));

    expect(library.projects.map((known) => known.id)).toEqual(['project-2', 'project-1']);
    expect(library.latest()?.id).toBe('project-2');

    const updated = [project('project-2'), project('project-1')];
    channel.deliver([{ item: { projects: updated }, kind: 'item' }]);

    expect(library.projects).toEqual(updated);
  });

  it('forget the projects as the plugin turns off', async () => {
    const { context, library } = await followedLibrary();
    channel.deliver([{ item: { projects: [project('project-1')] }, kind: 'item' }]);

    context.turnOff();

    expect(library.projects).toEqual([]);
    expect(library.status).toBe('loading');
    expect(core.unsubscribe).toHaveBeenCalled();
  });
});
