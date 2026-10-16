-- Whether every operation of a conversation runs without asking (ADR-0029;
-- docs/features/system-tools/operations-and-approvals.md, full access). The
-- owner switches it on a conversation, and a new conversation takes the
-- Tools settings' default. Task conversations follow their parent, so theirs
-- stays off. Conversations from before full access start with it off.
ALTER TABLE runtime_conversations
  ADD COLUMN full_access INTEGER NOT NULL DEFAULT 0 CHECK (full_access IN (0, 1));
