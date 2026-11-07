-- The owner's custom agents (ADR-0059): each is a JSON document under its key,
-- which never changes.
CREATE TABLE runtime_agents (
  key TEXT PRIMARY KEY NOT NULL
    CHECK (length(key) BETWEEN 8 AND 128 AND key LIKE 'custom.%'),
  document TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
) STRICT;
