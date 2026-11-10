-- The journal of installs of updates (ADR-0063). A record is committed before
-- Mannie quits to install a version, and the next start settles it. The
-- failure is an error text of the installer for the log.
CREATE TABLE updates_operations (
  id TEXT PRIMARY KEY NOT NULL,
  from_version TEXT NOT NULL,
  to_version TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('executing', 'completed', 'failed')),
  failure TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
) STRICT;
