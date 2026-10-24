-- The homes of conversations (ADR-0051): the attachment that a conversation
-- belongs to, such as the notebook of a notebook's chat, which its plugin
-- lists instead of the Chat list and which stays attached. Both columns are
-- set or neither is, and a conversation with a home has no project, which
-- the check of home_key enforces for every later insert and update.
ALTER TABLE runtime_conversations ADD COLUMN home_provider TEXT;
ALTER TABLE runtime_conversations ADD COLUMN home_key TEXT
  CHECK (
    (home_key IS NULL) = (home_provider IS NULL)
    AND (
      home_key IS NULL
      OR (length(home_key) BETWEEN 1 AND 256 AND project_id IS NULL)
    )
  );
