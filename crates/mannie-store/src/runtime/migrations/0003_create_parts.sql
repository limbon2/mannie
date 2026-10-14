-- The parts of the transcript (docs/features/chat/conversations.md). A part's
-- text is stored when the part finishes; while it streams, its text is in the
-- delta events of runtime_events.
CREATE TABLE runtime_parts (
  id TEXT PRIMARY KEY NOT NULL,
  conversation_id TEXT NOT NULL REFERENCES runtime_conversations (id) ON DELETE CASCADE,
  turn_id TEXT NOT NULL REFERENCES runtime_turns (id) ON DELETE CASCADE,
  -- The run that wrote the part, or NULL for the owner's message.
  run_id TEXT REFERENCES runtime_runs (id) ON DELETE CASCADE,
  -- The attempt of the run that wrote it, from 1.
  attempt INTEGER CHECK (attempt IS NULL OR attempt >= 1),
  -- The position within the turn, from 0.
  position INTEGER NOT NULL CHECK (position >= 0),
  kind TEXT NOT NULL,
  audience TEXT NOT NULL,
  state TEXT NOT NULL,
  text TEXT NOT NULL DEFAULT '',
  -- What else the kind needs, as a JSON document of the version in `version`.
  version INTEGER NOT NULL CHECK (version > 0),
  document TEXT NOT NULL CHECK (json_valid(document)),
  created_at INTEGER NOT NULL,
  finished_at INTEGER,
  UNIQUE (turn_id, position)
) STRICT;

CREATE INDEX runtime_parts_by_conversation ON runtime_parts (conversation_id, turn_id, position);

CREATE INDEX runtime_parts_by_run ON runtime_parts (run_id);

CREATE INDEX runtime_parts_streaming ON runtime_parts (state)
WHERE state = 'streaming';

-- Provider-private data of a part, such as encrypted reasoning, with the
-- binding it may be replayed to (ADR-0011, decision 6).
CREATE TABLE runtime_provider_items (
  part_id TEXT PRIMARY KEY NOT NULL REFERENCES runtime_parts (id) ON DELETE CASCADE,
  binding TEXT NOT NULL,
  version INTEGER NOT NULL CHECK (version > 0),
  item TEXT NOT NULL CHECK (json_valid(item))
) STRICT;
