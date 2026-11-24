-- Owner-initiated runs of installed programs from plugin views, journaled
-- before they run (ADR-0078, decision 1). Arguments are hashed, never stored
-- in plain text.
CREATE TABLE tools_program_operations (
  id TEXT PRIMARY KEY NOT NULL,
  plugin TEXT NOT NULL,
  project_id TEXT NOT NULL,
  program TEXT NOT NULL,
  action TEXT NOT NULL,
  arguments_digest TEXT NOT NULL,
  effects TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('executing', 'completed', 'failed', 'uncertain')),
  exit_code INTEGER,
  failure TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
) STRICT;
