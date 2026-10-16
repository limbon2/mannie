-- Tasks of workers (docs/features/workers/delegation.md, data; ADR-0027 and
-- ADR-0028). A task that the master delegates to a worker has a part in the
-- parent's reply but no turn on the master, where its conversation only
-- names the worker; a task that a worker runs for the master has a turn but
-- no part on the worker. SQLite cannot loosen NOT NULL in place, so the table
-- is built again with the same rows. No table refers to it.
CREATE TABLE runtime_tasks_new (
  id TEXT PRIMARY KEY NOT NULL,
  conversation_id TEXT NOT NULL REFERENCES runtime_conversations (id) ON DELETE CASCADE,
  -- The parent's part that assigned the task; none for a task that another
  -- machine assigned.
  part_id TEXT REFERENCES runtime_parts (id) ON DELETE CASCADE,
  -- The turn that works on the task; none for a task that runs on a worker.
  turn_id TEXT REFERENCES runtime_turns (id) ON DELETE CASCADE,
  follow_up INTEGER NOT NULL CHECK (follow_up IN (0, 1)),
  background INTEGER NOT NULL CHECK (background IN (0, 1)),
  state TEXT NOT NULL,
  version INTEGER NOT NULL CHECK (version > 0),
  ending TEXT CHECK (ending IS NULL OR json_valid(ending)),
  created_at INTEGER NOT NULL,
  ended_at INTEGER,
  -- On the master: the worker that runs the task, the provider profile whose
  -- credential the master leases to it, the version of the worker's state
  -- that the master stored last, whether the worker stored the task, and
  -- when the owner or the parent's run asked to cancel it.
  worker_id TEXT,
  profile_id TEXT,
  remote_version INTEGER,
  submission TEXT,
  cancel_requested_at INTEGER,
  -- On a worker: what the master assigned, as a JSON document, and the
  -- version of the task's latest stored state.
  assignment TEXT CHECK (assignment IS NULL OR json_valid(assignment)),
  state_version INTEGER NOT NULL DEFAULT 0,
  CHECK (part_id IS NOT NULL OR assignment IS NOT NULL),
  CHECK (turn_id IS NOT NULL OR worker_id IS NOT NULL)
) STRICT;

INSERT INTO runtime_tasks_new (
  id, conversation_id, part_id, turn_id, follow_up, background, state, version, ending,
  created_at, ended_at
)
SELECT
  id, conversation_id, part_id, turn_id, follow_up, background, state, version, ending,
  created_at, ended_at
FROM runtime_tasks;

DROP TABLE runtime_tasks;

ALTER TABLE runtime_tasks_new RENAME TO runtime_tasks;

-- A conversation works on one task at a time.
CREATE UNIQUE INDEX runtime_tasks_one_active ON runtime_tasks (conversation_id)
WHERE state IN ('accepted', 'running');

CREATE INDEX runtime_tasks_by_part ON runtime_tasks (part_id);

CREATE INDEX runtime_tasks_by_turn ON runtime_tasks (turn_id);

CREATE INDEX runtime_tasks_by_worker ON runtime_tasks (worker_id)
WHERE worker_id IS NOT NULL;

CREATE INDEX runtime_tasks_by_state_version ON runtime_tasks (state_version)
WHERE assignment IS NOT NULL;
