// The messages of the feature Databases (plugins/workers/docs/databases.md;
// ADR-0019): every error of the service, refusal of a file, state, kind of
// table and comparison has a catalog message of its own, SQLite's messages
// and names enter only as arguments, and the host's refusals name the
// feature.
import { describe, expect, it } from 'vitest';

import type {
  DatabaseReach,
  FileRefusal,
  FileState,
  FilterOperator,
  HostDatabaseRefusal,
  SqliteFilesError,
  TableKind,
} from '../../../ui/generated/bindings.js';
import type { DatabasesFailure } from '../../../ui/types/databases.js';
import {
  databaseReachLabel,
  databasesFailureMessage,
  databaseStateLabel,
  databaseStatusLabel,
  databasesToolFailure,
  filesErrorMessage,
  isUsable,
  operatorLabel,
  tableKindLabel,
  unusableMessage,
  workerRefusalMessage,
} from '../../../ui/utils/databases-messages.js';

const ERRORS: SqliteFilesError[] = [
  { code: 'not_registered' },
  { code: 'already_registered', key: '0199e0f2-0000-7000-8000-0000000000ee' },
  { code: 'too_many_files' },
  { code: 'file_missing' },
  { code: 'file_changed' },
  { code: 'not_a_database' },
  { code: 'marked_read_only' },
  { code: 'file_read_only' },
  { code: 'invalid_name' },
  { code: 'invalid_filter' },
  { code: 'not_one_statement' },
  { code: 'too_many_statements' },
  { code: 'parameters', expected: 2, given: 1 },
  { code: 'invalid_value' },
  { code: 'invalid_statement', message: 'near "SELEC": syntax error' },
  { code: 'refused', message: 'not authorized' },
  { code: 'not_a_read', class: 'write' },
  { code: 'schema_unconfirmed' },
  { code: 'busy' },
  { code: 'timed_out' },
  { code: 'constraint', message: 'FOREIGN KEY constraint failed' },
  { code: 'row_changed' },
  { code: 'no_key' },
  { code: 'not_allowed' },
  { code: 'already_run' },
  { code: 'no_dialog' },
  { code: 'read_only_host' },
  { code: 'schema_too_large' },
  { code: 'failed' },
];

const REFUSALS: FileRefusal[] = ['link', 'network', 'companion', 'mannie_folder', 'not_a_database', 'not_a_file'];

describe('filesErrorMessage', () => {
  it('words every error of the service differently', () => {
    const messages = ERRORS.map((error) => filesErrorMessage(error));

    expect(new Set(messages).size).toBe(ERRORS.length);
  });

  it('words every refusal of a file differently', () => {
    const messages = REFUSALS.map((reason) => filesErrorMessage({ code: 'file_refused', reason }));

    expect(new Set(messages).size).toBe(REFUSALS.length);
  });

  it("passes SQLite's message and the counts of parameters as arguments", () => {
    expect(filesErrorMessage({ code: 'constraint', message: 'FOREIGN KEY constraint failed' })).toContain(
      'FOREIGN KEY constraint failed',
    );
    expect(filesErrorMessage({ code: 'parameters', expected: 3, given: 1 })).toMatch(/3.*1/u);
  });
});

describe('databasesFailureMessage', () => {
  it("words the plugin's own errors and the service's inside them", () => {
    const taken: DatabasesFailure = {
      error: { code: 'name_taken', description: '', origin: 'plugin' },
      kind: 'core',
    };
    const busy: DatabasesFailure = {
      error: { code: 'files', description: '', error: { code: 'busy' }, origin: 'plugin' },
      kind: 'core',
    };
    const off: DatabasesFailure = {
      error: { code: 'feature_off', description: '', feature: 'databases', origin: 'host', plugin: 'workers' },
      kind: 'core',
    };

    expect(databasesFailureMessage(taken)).not.toBe('');
    expect(databasesFailureMessage(busy)).toBe(filesErrorMessage({ code: 'busy' }));
    expect(databasesFailureMessage(off)).not.toBe('');
    expect(databasesFailureMessage({ kind: 'ipc' })).not.toBe('');
  });
});

describe('the labels of the view', () => {
  it('name every state, kind of table and comparison differently', () => {
    const states: FileState[] = ['available', 'missing', 'changed', 'unreadable'];
    const kinds: TableKind[] = ['table', 'view', 'virtual', 'shadow'];
    const operators: FilterOperator[] = [
      'equal',
      'not_equal',
      'less',
      'less_or_equal',
      'greater',
      'greater_or_equal',
      'like',
      'is_null',
      'is_not_null',
    ];

    expect(new Set(states.map((state) => databaseStateLabel(state))).size).toBe(states.length);
    expect(new Set(kinds.map((kind) => tableKindLabel(kind))).size).toBe(kinds.length);
    expect(new Set(operators.map((operator) => operatorLabel(operator))).size).toBe(operators.length);
  });

  it('says what the owner can do only for a database that cannot run statements', () => {
    expect(unusableMessage({ reach: 'reachable', state: 'available' })).toBeNull();
    expect(unusableMessage({ reach: 'reachable', state: 'changed' })).not.toBeNull();
    expect(unusableMessage({ reach: 'reachable', state: 'missing' })).not.toBeNull();
    expect(unusableMessage({ reach: 'worker_offline', state: 'available' })).not.toBeNull();
  });

  it("names a worker's reach before its file's state, and only a reached file is usable", () => {
    const reaches: DatabaseReach[] = ['reachable', 'worker_offline', 'worker_needs_update', 'not_allowed'];

    expect(new Set(reaches.map((reach) => databaseReachLabel(reach))).size).toBe(reaches.length);
    expect(databaseStatusLabel({ reach: 'worker_offline', state: 'missing' })).toBe(
      databaseReachLabel('worker_offline'),
    );
    expect(databaseStatusLabel({ reach: 'reachable', state: 'missing' })).toBe(databaseStateLabel('missing'));
    expect(isUsable({ reach: 'reachable', state: 'available' })).toBe(true);
    expect(isUsable({ reach: 'not_allowed', state: 'available' })).toBe(false);
  });

  it("words every refusal of a worker's ceiling differently, and the commands' worker errors", () => {
    const refusals: HostDatabaseRefusal[] = [
      'no_databases_on_host',
      'read_only_host',
      'outside_allowed_folders',
      'worker_folder',
    ];

    expect(new Set(refusals.map((refusal) => workerRefusalMessage(refusal))).size).toBe(refusals.length);
    expect(
      databasesFailureMessage({
        error: { code: 'host_refused', description: '', origin: 'plugin', refusal: 'read_only_host' },
        kind: 'core',
      }),
    ).toBe(workerRefusalMessage('read_only_host'));
    const workerCodes = ['worker_offline', 'worker_needs_update', 'needs_owner', 'answer_lost'] as const;
    const messages = workerCodes.map((code) =>
      databasesFailureMessage({ error: { code, description: '', origin: 'plugin' }, kind: 'core' }),
    );
    expect(new Set(messages).size).toBe(workerCodes.length);
  });
});

describe('databasesToolFailure', () => {
  it("words the tools' own codes and the service's without their prefix, and nothing else", () => {
    expect(databasesToolFailure('unknown_database')).not.toBeNull();
    expect(databasesToolFailure('busy')).toBe(filesErrorMessage({ code: 'busy' }));
    expect(databasesToolFailure('constraint')).not.toBeNull();
    expect(databasesToolFailure('sqlite_files.busy')).toBeNull();
    expect(databasesToolFailure('something_else')).toBeNull();
  });

  it("words the codes of a worker's write and of the core's refusals", () => {
    for (const code of [
      'worker_offline',
      'worker_needs_update',
      'worker_needs_owner',
      'worker_answer_lost',
      'read_only_host',
      'outside_allowed_folders',
      'database_read_only_host',
      'database_worker_facts',
      'database_schema_too_large',
      'database_statement_refused',
    ]) {
      expect(databasesToolFailure(code), code).not.toBeNull();
    }
  });
});
