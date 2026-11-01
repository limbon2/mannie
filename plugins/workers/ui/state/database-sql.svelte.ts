// The SQL tab's sessions (plugins/workers/docs/databases.md, the view): for
// each database, the statement that the owner writes, the result of the
// latest run and the statements of this session, kept while the window
// lives, so switching tabs keeps them. A run asks the core what the
// statement does first: a read runs with an ID that "Stop" ends, a write
// runs as an owner operation, and a change of the schema runs only after
// the owner confirmed it.
import { call } from '@mannie/plugin-sdk/ipc';
import { SvelteMap } from 'svelte/reactivity';

import { commands, type DatabaseView, type StatementRows } from '../generated/bindings.js';
import type { DatabasesFailure } from '../types/databases.js';
import { ownerChange } from '../utils/databases-changes.js';

/** The most statements that a session keeps. */
const MAX_HISTORY = 20;

/** The result of a run: the rows of a read, or what a change did. */
export type SqlResult =
  { changes: string; kind: 'changed'; rows: StatementRows | null } | { kind: 'rows'; rows: StatementRows };

/** One database's session of the SQL tab. */
export class SqlSession {
  /** The statement of a change of the schema that waits for the owner's confirmation. */
  confirming: string | null = $state(null);
  /** Why the latest run failed. */
  failure: DatabasesFailure | null = $state(null);
  /** The statements of this session, the latest first. */
  history: string[] = $state([]);
  result: SqlResult | null = $state(null);
  /** What runs now. */
  running: 'changing' | 'classifying' | 'reading' | null = $state(null);
  text = $state('');
  /** The ID of the read that runs, which "Stop" names. */
  #run: string | null = null;

  /** Runs the confirmed change of the schema; true once it ran, so the page reads the structure again. */
  async confirmSchema(database: DatabaseView): Promise<boolean> {
    const statement = this.confirming;
    this.confirming = null;
    return statement === null ? false : this.#change(database, statement, true);
  }

  /**
   * Runs the statement that the owner wrote. Returns true when a change of
   * the schema ran; one that needs the owner's confirmation waits in
   * `confirming`.
   */
  async start(database: DatabaseView): Promise<boolean> {
    const statement = this.text.trim();
    const isBlank = statement === '';
    const isRunning = this.running !== null;
    if (isBlank || isRunning) {
      return false;
    }
    this.failure = null;
    this.result = null;
    this.#remember(statement);
    this.running = 'classifying';
    const classified = await call(() => commands.classifyDatabaseStatement({ key: database.key, text: statement }));
    if (!classified.ok) {
      this.running = null;
      this.failure = classified.failure;
      return false;
    }
    switch (classified.data) {
      case 'read': {
        await this.#read(database, statement);
        return false;
      }
      case 'schema': {
        this.running = null;
        this.confirming = statement;
        return false;
      }
      case 'write': {
        await this.#change(database, statement, false);
        return false;
      }
    }
  }

  /** Stops the read that runs; one that ended stays as it is. */
  async stop(): Promise<void> {
    const run = this.#run;
    if (run !== null) {
      await call(() => commands.stopDatabaseRead({ run }));
    }
  }

  /** Writes a statement of the session's history into the editor again. */
  use(statement: string): void {
    this.text = statement;
  }

  async #change(database: DatabaseView, statement: string, schemaConfirmed: boolean): Promise<boolean> {
    this.running = 'changing';
    const change = ownerChange({
      kind: 'statements',
      schemaConfirmed,
      statements: [{ parameters: [], text: statement }],
    });
    const outcome = await call(() => commands.runDatabaseChange({ change, key: database.key }));
    this.running = null;
    if (!outcome.ok) {
      this.failure = outcome.failure;
      return false;
    }
    this.result = { changes: outcome.data.changes, kind: 'changed', rows: outcome.data.rows };
    return schemaConfirmed;
  }

  async #read(database: DatabaseView, statement: string): Promise<void> {
    const run = crypto.randomUUID();
    this.#run = run;
    this.running = 'reading';
    const outcome = await call(() =>
      commands.readDatabase({ key: database.key, run, statement: { parameters: [], text: statement } }),
    );
    if (this.#run !== run) {
      return;
    }
    this.#run = null;
    this.running = null;
    if (outcome.ok) {
      this.result = { kind: 'rows', rows: outcome.data };
    } else {
      this.failure = outcome.failure;
    }
  }

  #remember(statement: string): void {
    const others = this.history.filter((known) => known !== statement);
    this.history = [statement, ...others].slice(0, MAX_HISTORY);
  }
}

/** The sessions by database key. */
const sessions = new SvelteMap<string, SqlSession>();

/** The SQL tab's session of a database, which starts empty. */
export function sqlSession(key: string): SqlSession {
  let session = sessions.get(key);
  if (session === undefined) {
    session = new SqlSession();
    sessions.set(key, session);
  }
  return session;
}

/** Forgets every session, as the feature turns off. */
export function forgetSqlSessions(): void {
  sessions.clear();
}
