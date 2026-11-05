// What the owner does with registered databases (plugins/workers/docs/
// databases.md, registering a database): adding one through the system's
// file dialog or, on a worker, by its path (ADR-0057, decision 3), naming
// one, marking one read-only, accepting a file that replaced one, and
// removing one. The dialogs these actions open are mounted once, as the
// feature's overlay. The feature's commands go through the plugin's
// generated bindings (ADR-0034, decision 9).
import { call, type PluginOutcome } from '@mannie/plugin-sdk/ipc';
import { SvelteMap } from 'svelte/reactivity';

import { commands, type DatabasesError, type DatabaseView, type WorkerView } from '../generated/bindings.js';
import { m } from '../generated/messages.js';
import type { DatabasesFailure } from '../types/databases.js';
import { openDatabase, openDatabases, shownDatabaseKey } from '../utils/databases-places.js';

/** One opening of the dialog that adds a database, whose fields start empty. */
export class AddDatabaseRequest {
  /** Whether the system's file dialog is open or the core or the worker is registering the file. */
  adding = $state(false);
  /** Why the latest attempt failed. */
  failure: DatabasesFailure | null = $state(null);
  /** The owner's name for the database; empty takes the file's name. */
  name = $state('');
  /** The file's absolute path on the worker's host, for a database on a worker. */
  path = $state('');
  readOnly = $state(false);
  /** The worker whose host holds the file, or `null` for this computer. */
  readonly worker: WorkerView | null;

  constructor(worker: WorkerView | null) {
    this.worker = worker;
  }
}

/** One opening of the rename dialog, which starts with the database's name. */
export class RenameDatabaseRequest {
  readonly database: DatabaseView;
  /** Why the latest attempt failed. */
  failure: DatabasesFailure | null = $state(null);
  name = $state('');
  saving = $state(false);

  constructor(database: DatabaseView) {
    this.database = database;
    this.name = database.name;
  }
}

/** One opening of the removal confirmation. */
export class RemoveDatabaseRequest {
  readonly database: DatabaseView;
  /** Why the latest attempt failed. */
  failure: DatabasesFailure | null = $state(null);
  removing = $state(false);

  constructor(database: DatabaseView) {
    this.database = database;
  }
}

/** The owner's actions on databases, and the dialogs they open. */
export class DatabaseActions {
  /** The latest dialog that adds a database, which stays while it closes. */
  adding: AddDatabaseRequest | null = $state(null);
  addOpen = $state(false);
  /** Why the latest mark or acceptance of a database failed, by its key, which its page shows. */
  readonly failures = new SvelteMap<string, DatabasesFailure>();
  /** The latest removal confirmation, which stays while it closes. */
  removing: RemoveDatabaseRequest | null = $state(null);
  removeOpen = $state(false);
  /** The latest rename dialog, which stays while it closes. */
  renaming: RenameDatabaseRequest | null = $state(null);
  renameOpen = $state(false);

  /** Accepts the file that is at a database's path now, as its page or its row's menu asks. */
  async accept(database: DatabaseView): Promise<void> {
    this.failures.delete(database.key);
    const outcome = await call(() => commands.acceptDatabase({ key: database.key }));
    if (!outcome.ok) {
      this.failures.set(database.key, outcome.failure);
    }
  }

  /** Closes every dialog, as the feature turns off. */
  closeAll(): void {
    this.addOpen = false;
    this.renameOpen = false;
    this.removeOpen = false;
    this.failures.clear();
  }

  /**
   * Registers the database of an add dialog: on this computer, the file that
   * the owner chooses in the system's file dialog, and on a worker, the file
   * at the path that the owner typed. The dialog that asked closes once the
   * file is registered, and stays when the owner closed the file dialog.
   */
  async confirmAdd(request: AddDatabaseRequest): Promise<void> {
    request.adding = true;
    request.failure = null;
    const outcome = await register(request);
    request.adding = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }
    if (outcome.data === null) {
      return;
    }
    if (this.adding === request) {
      this.addOpen = false;
    }
    await openDatabase(outcome.data.key);
  }

  /** Forgets the database of a removal confirmation, which closes once the core forgot it. */
  async confirmRemove(request: RemoveDatabaseRequest): Promise<void> {
    request.removing = true;
    request.failure = null;
    const { key } = request.database;
    const outcome = await call(() => commands.removeDatabase({ key }));
    request.removing = false;
    const gone = isGone(outcome.ok ? null : outcome.failure);
    if (!outcome.ok && !gone) {
      request.failure = outcome.failure;
      return;
    }
    if (this.removing === request) {
      this.removeOpen = false;
    }
    if (shownDatabaseKey() === key) {
      await openDatabases();
    }
  }

  /** Renames the database of a rename dialog, which closes once the core kept the name. */
  async confirmRename(request: RenameDatabaseRequest): Promise<void> {
    request.saving = true;
    request.failure = null;
    const outcome = await call(() => commands.renameDatabase({ key: request.database.key, name: request.name }));
    request.saving = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }
    if (this.renaming === request) {
      this.renameOpen = false;
    }
  }

  /** Marks a database read-only, or lifts the mark. */
  async mark(database: DatabaseView, readOnly: boolean): Promise<void> {
    this.failures.delete(database.key);
    const outcome = await call(() => commands.markDatabase({ key: database.key, readOnly }));
    if (!outcome.ok) {
      this.failures.set(database.key, outcome.failure);
    }
  }

  /** Opens the dialog that adds a database on this computer, or on a worker. */
  startAdd(worker: WorkerView | null = null): void {
    this.adding = new AddDatabaseRequest(worker);
    this.addOpen = true;
  }

  /** Opens the removal confirmation of a database. */
  startRemove(database: DatabaseView): void {
    this.removing = new RemoveDatabaseRequest(database);
    this.removeOpen = true;
  }

  /** Opens the rename dialog of a database. */
  startRename(database: DatabaseView): void {
    this.renaming = new RenameDatabaseRequest(database);
    this.renameOpen = true;
  }
}

/** Registers the database of an add dialog, on this computer or on its worker. */
async function register(request: AddDatabaseRequest): Promise<PluginOutcome<DatabaseView | null, DatabasesError>> {
  const typed = request.name.trim();
  const name = typed === '' ? null : typed;
  const { readOnly, worker } = request;
  if (worker === null) {
    return call(() =>
      commands.addDatabase({
        allFilesFilter: m.workers_databases_filter_all(),
        databasesFilter: m.workers_databases_filter_databases(),
        name,
        readOnly,
      }),
    );
  }
  const path = request.path.trim();
  return call(() => commands.addWorkerDatabase({ name, path, readOnly, worker: worker.id }));
}

/** Whether a removal failed because the database was gone already, which counts as removed. */
function isGone(failure: DatabasesFailure | null): boolean {
  if (failure?.kind !== 'core') {
    return false;
  }
  const { error } = failure;
  if (error.origin !== 'plugin') {
    return false;
  }
  if (error.code !== 'files') {
    return false;
  }
  return error.error.code === 'not_registered';
}

/** The owner's actions on databases. */
export const databaseActions = new DatabaseActions();
