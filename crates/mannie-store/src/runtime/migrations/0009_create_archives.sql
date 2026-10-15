-- Outputs of tool calls that did not fit into the transcript
-- (docs/architecture/agent-runtime.md, context management). The model reads
-- them back in pages, and the owner opens them from the tool call. An archive
-- belongs to its tool call's part and is deleted with it.
CREATE TABLE runtime_archives (
  id TEXT PRIMARY KEY NOT NULL,
  conversation_id TEXT NOT NULL REFERENCES runtime_conversations (id) ON DELETE CASCADE,
  part_id TEXT NOT NULL REFERENCES runtime_parts (id) ON DELETE CASCADE,
  -- What it holds, such as stdout or stderr.
  name TEXT NOT NULL,
  content TEXT NOT NULL,
  bytes INTEGER NOT NULL CHECK (bytes >= 0),
  lines INTEGER NOT NULL CHECK (lines >= 0),
  created_at INTEGER NOT NULL
) STRICT;

CREATE INDEX runtime_archives_by_part ON runtime_archives (part_id);
