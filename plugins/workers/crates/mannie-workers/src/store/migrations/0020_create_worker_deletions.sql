-- The deletions of task conversations that wait for a worker that was offline
-- when the owner deleted their parent (docs/features/workers/delegation.md,
-- deleting). Removing the worker drops them.
CREATE TABLE workers_deletions (
  worker_id TEXT NOT NULL REFERENCES workers_paired (id) ON DELETE CASCADE,
  conversation_id TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (worker_id, conversation_id)
) STRICT, WITHOUT ROWID;
