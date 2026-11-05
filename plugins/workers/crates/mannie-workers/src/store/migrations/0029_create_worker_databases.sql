-- The databases that the owner registered on each paired worker's host, as
-- the worker listed them last (plugins/workers/docs/databases.md; ADR-0057,
-- decision 3). The master shows them while the worker is offline and keeps
-- no path. Removing the worker drops them.
CREATE TABLE workers_databases (
  worker_id TEXT NOT NULL REFERENCES workers_paired (id) ON DELETE CASCADE,
  key TEXT NOT NULL,
  name TEXT NOT NULL,
  file_name TEXT NOT NULL,
  read_only INTEGER NOT NULL CHECK (read_only IN (0, 1)),
  state TEXT NOT NULL,
  identity TEXT NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (worker_id, key)
) STRICT, WITHOUT ROWID;
