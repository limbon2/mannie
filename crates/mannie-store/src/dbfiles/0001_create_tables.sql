-- The owner's SQLite files on this computer that a plugin registered, which
-- the plugin reaches by their keys and never by their paths (ADR-0056,
-- decision 2).
CREATE TABLE dbfiles_registrations (
  plugin TEXT NOT NULL,
  key TEXT NOT NULL,
  name TEXT NOT NULL,
  path TEXT NOT NULL,
  identity TEXT NOT NULL,
  read_only INTEGER NOT NULL CHECK (read_only IN (0, 1)),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (plugin, key),
  UNIQUE (plugin, path)
) STRICT, WITHOUT ROWID;

-- The owner's own changes from a plugin's view, journaled before they run
-- (ADR-0056, decision 8): the statement's text and a digest of its
-- parameters, never their values.
CREATE TABLE dbfiles_operations (
  id TEXT PRIMARY KEY NOT NULL,
  plugin TEXT NOT NULL,
  key TEXT NOT NULL,
  statement TEXT NOT NULL,
  statement_cut INTEGER NOT NULL CHECK (statement_cut IN (0, 1)),
  parameters_digest TEXT NOT NULL,
  class TEXT NOT NULL CHECK (class IN ('write', 'schema')),
  state TEXT NOT NULL CHECK (state IN ('executing', 'completed', 'failed', 'uncertain')),
  changes INTEGER CHECK (changes IS NULL OR changes >= 0),
  failure TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
) STRICT;
