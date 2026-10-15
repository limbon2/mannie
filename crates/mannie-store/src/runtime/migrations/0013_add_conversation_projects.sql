-- The project of a conversation, or NULL for a general conversation. The
-- conversation's workspace is its project's folder, copied when the
-- conversation starts in the project or moves into it; a project's folder
-- never changes. The foreign key has no action on delete, so a project row
-- cannot go while a conversation refers to it: the runtime deletes the
-- project's conversations first.
ALTER TABLE runtime_conversations ADD COLUMN project_id TEXT REFERENCES runtime_projects (id);

-- Every folder that conversations already used as their workspace becomes a
-- project. The project takes the ID and the creation time of the earliest of
-- them: a UUIDv7 with the right time, and IDs of different kinds never meet
-- in one table.
INSERT INTO runtime_projects (id, name, folder, created_at, updated_at)
SELECT c.id, NULL, c.workspace, c.created_at, c.created_at
FROM runtime_conversations c
WHERE c.workspace IS NOT NULL
  AND NOT EXISTS (
    SELECT 1 FROM runtime_conversations e
    WHERE e.workspace = c.workspace
      AND (e.created_at < c.created_at OR (e.created_at = c.created_at AND e.id < c.id))
  );

UPDATE runtime_conversations
SET project_id = (
  SELECT p.id FROM runtime_projects p WHERE p.folder = runtime_conversations.workspace
)
WHERE workspace IS NOT NULL;

-- Deleting a project row checks the foreign key through this index, and the
-- runtime lists a project's conversations with it.
CREATE INDEX runtime_conversations_by_project ON runtime_conversations (project_id);
