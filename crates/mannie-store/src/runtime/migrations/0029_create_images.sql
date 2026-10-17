-- The bytes of the images that the owner's messages hold (ADR-0033). An image
-- belongs to its part, an owner part of the kind image, and is deleted with it
-- and with its conversation. The views that the UI receives never carry the
-- bytes; the UI reads them on demand. The formats are kept as names, so that a
-- later format is an addition.
CREATE TABLE runtime_images (
  part_id TEXT PRIMARY KEY NOT NULL REFERENCES runtime_parts (id) ON DELETE CASCADE,
  conversation_id TEXT NOT NULL REFERENCES runtime_conversations (id) ON DELETE CASCADE,
  format TEXT NOT NULL,
  data BLOB NOT NULL,
  thumbnail_format TEXT NOT NULL,
  thumbnail BLOB NOT NULL,
  created_at INTEGER NOT NULL
) STRICT;

CREATE INDEX runtime_images_by_conversation ON runtime_images (conversation_id);
