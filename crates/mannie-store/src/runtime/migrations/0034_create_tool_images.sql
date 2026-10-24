-- The images of tool results (ADR-0044). An image belongs to its tool call's
-- part, at its position among the result's images, and is deleted with the
-- part and with its conversation. The views that the UI receives never carry
-- the bytes; the UI reads them on demand. The formats are kept as names, so
-- that a later format is an addition.
CREATE TABLE runtime_tool_images (
  part_id TEXT NOT NULL REFERENCES runtime_parts (id) ON DELETE CASCADE,
  position INTEGER NOT NULL CHECK (position >= 0),
  conversation_id TEXT NOT NULL REFERENCES runtime_conversations (id) ON DELETE CASCADE,
  format TEXT NOT NULL,
  width INTEGER NOT NULL CHECK (width > 0),
  height INTEGER NOT NULL CHECK (height > 0),
  data BLOB NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (part_id, position)
) STRICT;

CREATE INDEX runtime_tool_images_by_conversation ON runtime_tool_images (conversation_id);
