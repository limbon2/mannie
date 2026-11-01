-- Rules for commands that a plugin's tool types into terminals (ADR-0053):
-- command prefixes that run without asking in every terminal of one host,
-- typed by one plugin, in every conversation. They live apart from the rules
-- of this machine's commands and of remote hosts, so a rule of one never
-- matches a command of another.
CREATE TABLE tools_terminal_rules (
  id TEXT PRIMARY KEY NOT NULL,
  -- The plugin whose tool types the commands.
  plugin TEXT NOT NULL,
  -- The host as a JSON document of the version in `version`, with the name
  -- that the owner gave a remote host when the rule was saved, for display.
  host TEXT NOT NULL CHECK (json_valid(host)),
  -- What decides which terminals the rule covers: the shell on this machine,
  -- or a remote host's plugin, target and endpoint, without its name.
  host_key TEXT NOT NULL,
  -- The words a command must start with, as a JSON array of the version in
  -- `version`.
  version INTEGER NOT NULL CHECK (version > 0),
  words TEXT NOT NULL CHECK (json_valid(words)),
  created_at INTEGER NOT NULL
) STRICT;

-- The same rule is saved once for the same plugin and host.
CREATE UNIQUE INDEX tools_terminal_rules_unique ON tools_terminal_rules (plugin, host_key, words);
