// @ts-check
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { grantLines } from '../../../scripts/plugins/lib/grants.mjs';

describe('grantLines', () => {
  it('lists every grant of a plugin, with its sensitive services marked', () => {
    const lines = grantLines(
      {
        folder: 'plugins/workers',
        id: 'workers',
        manifest: {
          crates: { exclusive: ['tonic'] },
          id: 'workers',
          native: { processes: 'Runs the updater.', sockets: 'The link.' },
          order: 10,
          programs: [{ crates: ['mannie-link-worker'], package: 'mannie-worker' }, { package: 'mannie-sync' }],
          rust: { entry: 'mannie-workers' },
          sdk: 1,
          secrets: { entries: ['token'], keyed: ['host'], legacy: ['link/master-key'] },
          services: { 'executor-host': 1, secrets: 1 },
        },
        validation: false,
      },
      { channels: ['subscribe_workers'], commands: ['pair_worker', 'remove_worker'], id: 'workers' },
      ['credentials', 'secrets'],
    );

    assert.deepEqual(lines, [
      'workers (plugins/workers)',
      '  Services: executor-host 1, secrets 1 (sensitive)',
      '  Secret entries: workers/token, link/master-key (a legacy name), workers/host/<key> (a keyed family)',
      '  Commands: pair_worker, remove_worker',
      '  Channels: subscribe_workers',
      '  Programs: mannie-worker, with mannie-link-worker; mannie-sync',
      '  Crates that only it may use: tonic',
      '  Native calls:',
      '    processes: Runs the updater.',
      '    sockets: The link.',
    ]);
  });

  it('says none for each kind of grant that a plugin does not have', () => {
    const lines = grantLines(
      {
        folder: 'tests/fixtures/plugins/example',
        id: 'example',
        manifest: { id: 'example', order: 90, rust: { entry: 'mannie-example' }, sdk: 1 },
        validation: true,
      },
      { channels: [], commands: [], id: 'example' },
      ['secrets'],
    );

    assert.deepEqual(lines, [
      'example (tests/fixtures/plugins/example, in validation builds only)',
      '  Services: none',
      '  Secret entries: none',
      '  Commands: none',
      '  Channels: none',
      '  Programs: none',
      '  Crates that only it may use: none',
      '  Native calls: none',
    ]);
  });
});
