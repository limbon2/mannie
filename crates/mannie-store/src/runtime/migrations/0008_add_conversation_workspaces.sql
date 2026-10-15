-- The workspace folder of a conversation (docs/features/system-tools/
-- operations-and-approvals.md, the workspace): a folder with its links
-- resolved, or NULL for none. Conversations from before system tools have
-- none.
ALTER TABLE runtime_conversations ADD COLUMN workspace TEXT;
