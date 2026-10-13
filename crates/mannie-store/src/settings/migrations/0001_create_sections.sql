-- Settings sections (docs/architecture/extension-points.md, settings
-- sections): one versioned JSON document per section key.
CREATE TABLE settings_sections (
  key TEXT PRIMARY KEY NOT NULL,
  version INTEGER NOT NULL CHECK (version > 0),
  document TEXT NOT NULL CHECK (json_valid(document)),
  updated_at INTEGER NOT NULL
) STRICT;
