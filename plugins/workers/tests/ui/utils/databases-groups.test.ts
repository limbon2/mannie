// The groups of the Databases view's sidebar (plugins/workers/docs/
// databases.md, the view; ADR-0057): this computer's databases first, then a
// group for each worker that allows databases or has some, which takes a new
// database only while the worker is connected and allows them.
import { describe, expect, it } from 'vitest';

import type { DatabaseView, WorkerView } from '../../../ui/generated/bindings.js';
import { databaseGroups } from '../../../ui/utils/databases-groups.js';

function database(key: string, machine: DatabaseView['machine']): DatabaseView {
  return {
    fileName: `${key}.db`,
    key,
    machine,
    name: key,
    reach: 'reachable',
    readOnly: false,
    state: 'available',
  };
}

function worker(id: string, status: WorkerView['status'], databases: boolean): WorkerView {
  return {
    address: `${id}.lan:7445`,
    chosenName: null,
    configuredName: id,
    connectedAt: null,
    details: { databases },
    id,
    keyPin: '0123456789ab',
    name: id,
    pairedAt: 0,
    status,
  };
}

describe('databaseGroups', () => {
  it("puts this computer first and a worker's databases under it, and leaves out a worker without them", () => {
    const groups = databaseGroups(
      [
        database('parts', { kind: 'this_computer' }),
        database('logs', { kind: 'worker', name: 'build', worker: 'build' }),
      ],
      [worker('build', 'offline', false), worker('quiet', 'connected', false), worker('ready', 'connected', true)],
    );

    expect(groups.map((group) => group.worker?.id ?? 'this computer')).toEqual(['this computer', 'build', 'ready']);
    expect(groups[0].databases.map((shown) => shown.key)).toEqual(['parts']);
    expect(groups[1].databases.map((shown) => shown.key)).toEqual(['logs']);
    expect(groups.map((group) => group.canAdd)).toEqual([true, false, true]);
  });
});
