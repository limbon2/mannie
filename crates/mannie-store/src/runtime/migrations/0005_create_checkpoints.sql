-- Checkpoints: summaries that stand in for older turns in the model's context
-- (docs/features/chat/conversations.md, context budget and compaction). The
-- transcript itself never changes.
CREATE TABLE runtime_checkpoints (
  id TEXT PRIMARY KEY NOT NULL,
  conversation_id TEXT NOT NULL REFERENCES runtime_conversations (id) ON DELETE CASCADE,
  -- The position of the last turn the checkpoint covers.
  covers_through_turn INTEGER NOT NULL CHECK (covers_through_turn >= 0),
  -- A hash of the covered text, so a change in it invalidates the checkpoint.
  content_hash TEXT NOT NULL,
  summary TEXT NOT NULL,
  version INTEGER NOT NULL CHECK (version > 0),
  document TEXT NOT NULL CHECK (json_valid(document)),
  created_at INTEGER NOT NULL
) STRICT;

CREATE INDEX runtime_checkpoints_latest ON runtime_checkpoints (
  conversation_id, covers_through_turn DESC, created_at DESC
);
