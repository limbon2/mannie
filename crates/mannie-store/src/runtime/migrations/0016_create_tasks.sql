-- Tasks (docs/features/child-agents/tasks.md, the contract): each start or
-- follow-up of a child agent's work. The task's ID is the one that the
-- parent's run chose and stored in its part before the task existed, so
-- creating a task twice creates it once.
--
-- The state column has no CHECK constraint, like the other states of the
-- runtime: the code checks it.
CREATE TABLE runtime_tasks (
  id TEXT PRIMARY KEY NOT NULL,
  -- The child's conversation, which the task's turn belongs to.
  conversation_id TEXT NOT NULL REFERENCES runtime_conversations (id) ON DELETE CASCADE,
  -- The parent's part that assigned the task.
  part_id TEXT NOT NULL REFERENCES runtime_parts (id) ON DELETE CASCADE,
  -- The child's turn that works on the task.
  turn_id TEXT NOT NULL REFERENCES runtime_turns (id) ON DELETE CASCADE,
  follow_up INTEGER NOT NULL CHECK (follow_up IN (0, 1)),
  background INTEGER NOT NULL CHECK (background IN (0, 1)),
  state TEXT NOT NULL,
  -- How the task ended: its state, the child's report and the failure, as a
  -- JSON document of the version in `version`.
  version INTEGER NOT NULL CHECK (version > 0),
  ending TEXT CHECK (ending IS NULL OR json_valid(ending)),
  created_at INTEGER NOT NULL,
  ended_at INTEGER
) STRICT;

-- A child works on one task at a time.
CREATE UNIQUE INDEX runtime_tasks_one_active ON runtime_tasks (conversation_id)
WHERE state IN ('accepted', 'running');

CREATE INDEX runtime_tasks_by_part ON runtime_tasks (part_id);

CREATE INDEX runtime_tasks_by_turn ON runtime_tasks (turn_id);
