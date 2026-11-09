// The structure of each database that the view shows (plugins/workers/docs/
// databases.md, the view): its tables, which the sidebar lists under the
// database, and what the page's Structure and Data tabs show. The sidebar
// and the page read the same structure, and a change of the schema from the
// SQL tab reads it again for both. A read keeps the structure it replaces
// until it ends, and a slower earlier read never covers a newer one.
import { call } from '@mannie/plugin-sdk/ipc';
import { SvelteMap } from 'svelte/reactivity';

import { commands, type DatabaseStructure } from '../generated/bindings.js';
import type { DatabasesFailure } from '../types/databases.js';

/** What the view knows of one database's structure. */
export interface StructureRead {
  /** Why the latest read failed, while no read runs. */
  readonly failure: DatabasesFailure | null;
  /** Whether a read runs. */
  readonly loading: boolean;
  /** The structure that the latest successful read gave, or `null` before one. */
  readonly structure: DatabaseStructure | null;
}

const NOT_READ: StructureRead = { failure: null, loading: false, structure: null };

/** The structures of the databases, by key. */
export class DatabaseStructures {
  readonly #reads = new SvelteMap<string, StructureRead>();
  /** Counts the reads of each database, so that only the newest one lands. */
  readonly #generations = new SvelteMap<string, number>();

  /** What the view knows of a database's structure. */
  get(key: string): StructureRead {
    return this.#reads.get(key) ?? NOT_READ;
  }

  /** Reads a database's structure unless it was read or is being read. */
  ensure(key: string): void {
    const known = this.#reads.get(key);
    if (known === undefined) {
      void this.load(key);
    }
  }

  /** Forgets a database's structure, such as after it was removed. */
  forget(key: string): void {
    this.#reads.delete(key);
    this.#generations.delete(key);
  }

  /** Reads a database's structure again, such as after a change of its schema. */
  async load(key: string): Promise<void> {
    const generation = (this.#generations.get(key) ?? 0) + 1;
    this.#generations.set(key, generation);
    this.#reads.set(key, { failure: null, loading: true, structure: this.get(key).structure });
    const outcome = await call(() => commands.databaseStructure({ key }));
    if (this.#generations.get(key) !== generation) {
      return;
    }
    const structure = outcome.ok ? outcome.data : this.get(key).structure;
    const failure = outcome.ok ? null : outcome.failure;
    this.#reads.set(key, { failure, loading: false, structure });
  }
}

/** The window's structures of the databases. */
export const databaseStructures = new DatabaseStructures();
