-- The draft conversation of each place where a new conversation starts
-- (ADR-0068): 1 while it has no message, which keeps it out of the
-- conversation list. Every conversation from before drafts is 0. A place, the
-- general conversations, a project or a home item, has at most one draft.
ALTER TABLE runtime_conversations ADD COLUMN draft INTEGER NOT NULL DEFAULT 0
  CHECK (draft IN (0, 1));
CREATE UNIQUE INDEX runtime_conversations_draft_place
  ON runtime_conversations (
    COALESCE(project_id, ''),
    COALESCE(home_provider, ''),
    COALESCE(home_key, '')
  )
  WHERE draft = 1;
