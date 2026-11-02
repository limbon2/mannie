// The groups of the Databases view's sidebar (plugins/workers/docs/
// databases.md, the view): this computer's databases, then each paired
// worker's, in the order of pairing. A worker shows while it allows
// databases or the list has databases on it, and the owner adds one on it
// while it is connected and allows them (ADR-0057, decision 3).
import type { DatabaseView, WorkerView } from '../generated/bindings.js';

/** One group of the sidebar: this computer's databases, or one worker's. */
export interface DatabaseGroup {
  /** Whether the owner can add a database to the group now. */
  canAdd: boolean;
  /** The group's databases, by name. */
  databases: DatabaseView[];
  /** The worker, or `null` for this computer. */
  worker: WorkerView | null;
}

/** The groups of the sidebar for these databases and workers. */
export function databaseGroups(databases: readonly DatabaseView[], workers: readonly WorkerView[]): DatabaseGroup[] {
  const groups: DatabaseGroup[] = [
    {
      canAdd: true,
      databases: databases.filter((database) => database.machine.kind === 'this_computer'),
      worker: null,
    },
  ];
  for (const worker of workers) {
    const onWorker = databases.filter((database) => workerOf(database) === worker.id);
    const allows = worker.details.databases === true;
    const isShown = allows || onWorker.length > 0;
    if (isShown) {
      groups.push({ canAdd: allows && worker.status === 'connected', databases: onWorker, worker });
    }
  }
  return groups;
}

/** The worker whose host holds a database, or `null` for this computer. */
function workerOf(database: DatabaseView): string | null {
  return database.machine.kind === 'worker' ? database.machine.worker : null;
}
