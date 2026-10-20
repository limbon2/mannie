-- Rules for runs of installed programs that plugins' manifests declare
-- (ADR-0038): leading words of a program's name and arguments that run
-- without asking in every conversation, for one plugin's program and with
-- the effects of the run that the rule was saved from. They live apart from
-- the rules of commands, so a rule of one never matches a run of the other.
CREATE TABLE tools_program_rules (
  id TEXT PRIMARY KEY NOT NULL,
  -- The plugin whose tool runs the program, the program's name in the
  -- plugin's manifest and the path that the core found, which together name
  -- what the rule covers.
  plugin TEXT NOT NULL,
  name TEXT NOT NULL,
  program TEXT NOT NULL,
  -- The words that a run's name and arguments must start with, and the keys
  -- of the effects that the rule covers, as JSON arrays of the version in
  -- `version`.
  version INTEGER NOT NULL CHECK (version > 0),
  words TEXT NOT NULL CHECK (json_valid(words)),
  effects TEXT NOT NULL CHECK (json_valid(effects)),
  -- The folder the working directory must lie in, or NULL for anywhere.
  folder TEXT,
  created_at INTEGER NOT NULL
) STRICT;

-- The same rule is saved once for the same program.
CREATE UNIQUE INDEX tools_program_rules_unique ON tools_program_rules (
  plugin,
  name,
  program,
  words,
  effects,
  COALESCE(folder, '')
);
