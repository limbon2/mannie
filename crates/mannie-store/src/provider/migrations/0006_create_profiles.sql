-- Provider profiles (docs/features/chat/providers.md). The credential lives in
-- the operating system's credential store; the table holds only the name of
-- its entry (ADR-0010).
CREATE TABLE provider_profiles (
  id TEXT PRIMARY KEY NOT NULL,
  name TEXT NOT NULL,
  interface TEXT NOT NULL,
  base_url TEXT NOT NULL,
  model TEXT NOT NULL,
  credential_ref TEXT,
  is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
  -- Increases when the interface, the base URL, the model or the credential
  -- changes. Provider-private data is bound to it (ADR-0011).
  config_revision INTEGER NOT NULL DEFAULT 1 CHECK (config_revision >= 1),
  -- The limits, the reasoning effort and what the provider test found out, as
  -- a JSON document of the version in `version`.
  version INTEGER NOT NULL CHECK (version > 0),
  document TEXT NOT NULL CHECK (json_valid(document)),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
) STRICT;

-- At most one default profile.
CREATE UNIQUE INDEX provider_profiles_one_default ON provider_profiles (is_default)
WHERE is_default = 1;

-- Names are unique regardless of case; the code also compares non-ASCII case.
CREATE UNIQUE INDEX provider_profiles_name ON provider_profiles (name COLLATE NOCASE);
