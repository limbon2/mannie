// The structure of each database that the view shows (plugins/workers/docs/
// databases.md, the view): read once for the sidebar and the page, read
// again on request while the older structure stays, a failure kept beside
// it, and a slower earlier read never covering a newer one.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { DatabaseStructure } from '../../../ui/generated/bindings.js';
import { DatabaseStructures } from '../../../ui/state/database-structures.svelte.js';

const core = vi.hoisted(() => ({ databaseStructure: vi.fn() }));

vi.mock('../../../ui/generated/bindings.js', () => ({
  commands: { databaseStructure: core.databaseStructure },
}));

/** A structure with tables of these names. */
function structureOf(...names: string[]): DatabaseStructure {
  return {
    indexes: [],
    tables: names.map((name) => ({
      columns: [],
      foreignKeys: [],
      kind: 'table',
      name,
      sql: null,
      strict: false,
      withoutRowid: false,
    })),
    triggers: [],
  };
}

const FAILED = { error: { code: 'not_found', description: '', origin: 'plugin' }, status: 'error' };

beforeEach(() => {
  core.databaseStructure.mockReset();
});

describe('DatabaseStructures', () => {
  it('reads a structure once for every reader that ensures it', async () => {
    core.databaseStructure.mockResolvedValue({ data: structureOf('parts'), status: 'ok' });
    const structures = new DatabaseStructures();

    structures.ensure('db');
    structures.ensure('db');
    expect(structures.get('db').loading).toBe(true);
    await vi.waitFor(() => {
      expect(structures.get('db').loading).toBe(false);
    });

    expect(core.databaseStructure).toHaveBeenCalledTimes(1);
    expect(structures.get('db').structure?.tables.map((table) => table.name)).toEqual(['parts']);
  });

  it('keeps the older structure while it reads again, and beside a failure', async () => {
    core.databaseStructure.mockResolvedValueOnce({ data: structureOf('parts'), status: 'ok' });
    const structures = new DatabaseStructures();
    await structures.load('db');

    core.databaseStructure.mockResolvedValueOnce(FAILED);
    const again = structures.load('db');
    expect(structures.get('db').structure).not.toBeNull();
    await again;

    expect(structures.get('db').failure).not.toBeNull();
    expect(structures.get('db').structure?.tables.map((table) => table.name)).toEqual(['parts']);
  });

  it('lets only the newest read land', async () => {
    let finishFirst: (value: unknown) => void = () => undefined;
    core.databaseStructure
      .mockReturnValueOnce(
        new Promise((resolve) => {
          finishFirst = resolve;
        }),
      )
      .mockResolvedValueOnce({ data: structureOf('new'), status: 'ok' });
    const structures = new DatabaseStructures();

    const first = structures.load('db');
    await structures.load('db');
    finishFirst({ data: structureOf('old'), status: 'ok' });
    await first;

    expect(structures.get('db').structure?.tables.map((table) => table.name)).toEqual(['new']);
  });

  it('forgets a removed database', async () => {
    core.databaseStructure.mockResolvedValue({ data: structureOf('parts'), status: 'ok' });
    const structures = new DatabaseStructures();
    await structures.load('db');

    structures.forget('db');

    expect(structures.get('db').structure).toBeNull();
  });
});
