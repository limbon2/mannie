-- Task conversations (docs/features/child-agents/tasks.md, task
-- conversations): the conversation of a child agent has a parent
-- conversation, a number among the parent's children and the name of its
-- agent definition. The owner's conversations have none of them.
--
-- Deleting a parent deletes its task conversations, and with them their
-- turns, parts, events and archives. A task conversation belongs to no
-- project; its runs work in its parent's workspace.
ALTER TABLE runtime_conversations ADD COLUMN parent_conversation_id TEXT
REFERENCES runtime_conversations (id) ON DELETE CASCADE;

ALTER TABLE runtime_conversations ADD COLUMN task_number INTEGER CHECK (
  task_number IS NULL OR task_number > 0
);

ALTER TABLE runtime_conversations ADD COLUMN task_agent TEXT;

-- A number names one child of its parent. The parent's run assigns them, and
-- deleting a parent row checks the foreign key through this index.
CREATE UNIQUE INDEX runtime_conversations_task_numbers ON runtime_conversations (
  parent_conversation_id, task_number
)
WHERE parent_conversation_id IS NOT NULL;
