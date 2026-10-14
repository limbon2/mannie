-- Conversations, their turns and the runs that answer them
-- (docs/features/chat/conversations.md). The runtime keeps the owner's
-- conversations on the master, and later the task conversations of child
-- agents and workers, so the tables carry the runtime's prefix.
--
-- Columns that name a state or a kind have no CHECK constraint: features add
-- values, and SQLite cannot change a constraint without rebuilding the table.
-- The code checks them instead.
CREATE TABLE runtime_conversations (
  id TEXT PRIMARY KEY NOT NULL,
  title TEXT NOT NULL,
  title_source TEXT NOT NULL,
  -- The provider profile the owner chose, or NULL for the default profile.
  -- The profile lives in another module's table, so there is no foreign key.
  profile_id TEXT,
  -- The sequence number of the latest event.
  last_seq INTEGER NOT NULL DEFAULT 0 CHECK (last_seq BETWEEN 0 AND 4294967295),
  -- The runtime's context ledger and the usage the view shows, as JSON
  -- documents of the version in `context_version`.
  context_version INTEGER NOT NULL DEFAULT 1 CHECK (context_version > 0),
  context TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(context)),
  context_usage TEXT CHECK (context_usage IS NULL OR json_valid(context_usage)),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
) STRICT;

CREATE INDEX runtime_conversations_recent ON runtime_conversations (updated_at DESC, id DESC);

CREATE TABLE runtime_turns (
  id TEXT PRIMARY KEY NOT NULL,
  conversation_id TEXT NOT NULL REFERENCES runtime_conversations (id) ON DELETE CASCADE,
  position INTEGER NOT NULL CHECK (position >= 0),
  -- The run whose reply the turn shows: the latest one.
  current_run_id TEXT,
  created_at INTEGER NOT NULL,
  UNIQUE (conversation_id, position)
) STRICT;

CREATE TABLE runtime_runs (
  id TEXT PRIMARY KEY NOT NULL,
  conversation_id TEXT NOT NULL REFERENCES runtime_conversations (id) ON DELETE CASCADE,
  turn_id TEXT NOT NULL REFERENCES runtime_turns (id) ON DELETE CASCADE,
  -- The agent definition that runs it.
  agent TEXT NOT NULL,
  profile_id TEXT NOT NULL,
  -- The profile and its configuration revision when the run started.
  -- Provider-private data is replayed only to the same binding (ADR-0011).
  binding TEXT NOT NULL,
  state TEXT NOT NULL,
  -- What the run does while it runs, and how it ended, as JSON documents of
  -- the version in `version`.
  version INTEGER NOT NULL CHECK (version > 0),
  activity TEXT CHECK (activity IS NULL OR json_valid(activity)),
  outcome TEXT CHECK (outcome IS NULL OR json_valid(outcome)),
  created_at INTEGER NOT NULL,
  started_at INTEGER,
  ended_at INTEGER
) STRICT;

-- A conversation has at most one active run.
CREATE UNIQUE INDEX runtime_runs_one_active ON runtime_runs (conversation_id)
WHERE state IN ('queued', 'running');

CREATE INDEX runtime_runs_by_turn ON runtime_runs (turn_id);
