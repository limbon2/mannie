-- Rules for commands on remote hosts (ADR-0036): command prefixes that run
-- without asking on one host, reached at one endpoint, in every
-- conversation. They live apart from the rules of this machine, so a rule of
-- one never matches a command of the other.
CREATE TABLE tools_remote_rules (
  id TEXT PRIMARY KEY NOT NULL,
  -- The plugin that reaches the host, its key for the host and where commands
  -- run there, which together name what the rule covers.
  plugin TEXT NOT NULL,
  target TEXT NOT NULL,
  endpoint TEXT NOT NULL,
  -- The host's name when the owner saved the rule, for display only.
  label TEXT NOT NULL,
  -- The words a command must start with, as a JSON array of the version in
  -- `version`.
  version INTEGER NOT NULL CHECK (version > 0),
  words TEXT NOT NULL CHECK (json_valid(words)),
  created_at INTEGER NOT NULL
) STRICT;

-- The same rule is saved once for the same host.
CREATE UNIQUE INDEX tools_remote_rules_unique ON tools_remote_rules (plugin, target, endpoint, words);
