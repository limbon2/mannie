-- The event log of each conversation (ADR-0012). Events carry the
-- conversation's sequence numbers. A finished part's delta events are deleted
-- when the part finishes, because its finished event carries the whole text.
CREATE TABLE runtime_events (
  conversation_id TEXT NOT NULL REFERENCES runtime_conversations (id) ON DELETE CASCADE,
  seq INTEGER NOT NULL CHECK (seq > 0),
  kind TEXT NOT NULL,
  -- The part the event concerns, for the events of a part.
  part_id TEXT,
  version INTEGER NOT NULL CHECK (version > 0),
  body TEXT NOT NULL CHECK (json_valid(body)),
  created_at INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, seq)
) STRICT, WITHOUT ROWID;

CREATE INDEX runtime_events_deltas ON runtime_events (part_id, seq)
WHERE kind = 'part_delta';
