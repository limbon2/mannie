-- The workers that the master paired with (docs/features/workers/
-- pairing-and-link.md, data). The pin is the SHA-256 of the worker's public
-- key, which the master accepts from this worker only.
CREATE TABLE workers_paired (
  id TEXT PRIMARY KEY NOT NULL,
  name TEXT NOT NULL,
  address TEXT NOT NULL,
  key_pin TEXT NOT NULL UNIQUE,
  -- What the worker said about itself when it last connected: its purpose,
  -- host facts, ceiling and version, as a JSON document of the version in
  -- `version`.
  version INTEGER NOT NULL CHECK (version > 0),
  details TEXT NOT NULL CHECK (json_valid(details)),
  paired_at INTEGER NOT NULL,
  connected_at INTEGER,
  updated_at INTEGER NOT NULL
) STRICT;
