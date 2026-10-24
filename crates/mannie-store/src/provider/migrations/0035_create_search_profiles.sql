-- Search profiles (ADR-0047): one embedding or rerank model on one server.
-- The credential lives in the operating system's credential store; the table
-- holds only the name of its entry (ADR-0010).
CREATE TABLE provider_search_profiles (
  id TEXT PRIMARY KEY NOT NULL,
  name TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('embeddings', 'rerank')),
  base_url TEXT NOT NULL,
  model TEXT NOT NULL,
  credential_ref TEXT,
  -- Increases when the kind, the base URL, the model, the credential or a
  -- prefix changes.
  config_revision INTEGER NOT NULL DEFAULT 1 CHECK (config_revision >= 1),
  -- The prefixes and what the test found out, as a JSON document of the
  -- version in `version`.
  version INTEGER NOT NULL CHECK (version > 0),
  document TEXT NOT NULL CHECK (json_valid(document)),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
) STRICT;

-- Names are unique regardless of case; the code also compares non-ASCII case.
CREATE UNIQUE INDEX provider_search_profiles_name ON provider_search_profiles (name COLLATE NOCASE);
