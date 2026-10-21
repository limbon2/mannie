-- Metadata only. No transcript content is copied into the export journal.
CREATE TABLE exports_operations (
  id TEXT PRIMARY KEY NOT NULL,
  conversation_id TEXT NOT NULL,
  destination TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('executing', 'completed', 'failed', 'uncertain')),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
) STRICT;
