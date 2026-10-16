-- The executor that serves a task conversation, for the conversation of a
-- task that a plugin's executor runs (ADR-0031, decision 6): its kind, its
-- target, the target's name when the conversation started, and what the text
-- that the model reads calls the target. The conversation of a task that 0.2.0
-- delegated to a worker gets the Workers plugin's executor; the worker's
-- columns stay unused.
ALTER TABLE runtime_conversations ADD COLUMN task_executor_kind TEXT;

ALTER TABLE runtime_conversations ADD COLUMN task_executor_target TEXT;

ALTER TABLE runtime_conversations ADD COLUMN task_executor_name TEXT;

ALTER TABLE runtime_conversations ADD COLUMN task_executor_noun TEXT;

UPDATE runtime_conversations
SET
  task_executor_kind = 'workers',
  task_executor_target = task_worker_id,
  task_executor_name = coalesce(task_worker_name, ''),
  task_executor_noun = 'the worker'
WHERE task_worker_id IS NOT NULL;
