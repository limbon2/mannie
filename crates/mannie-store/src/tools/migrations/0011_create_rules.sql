-- Command rules: command prefixes that run without asking, in every
-- conversation (docs/features/system-tools/operations-and-approvals.md,
-- rules).
CREATE TABLE tools_rules (
  id TEXT PRIMARY KEY NOT NULL,
  -- The words a command must start with, as a JSON array of the version in
  -- `version`.
  version INTEGER NOT NULL CHECK (version > 0),
  words TEXT NOT NULL CHECK (json_valid(words)),
  -- The folder the working directory must lie in, or NULL for anywhere.
  folder TEXT,
  created_at INTEGER NOT NULL
) STRICT;

-- The same rule is saved once.
CREATE UNIQUE INDEX tools_rules_unique ON tools_rules (words, COALESCE(folder, ''));
