-- The conversation of a task that runs on a worker (docs/features/workers/
-- delegation.md, worker task conversations): a row on the master with the
-- parent, the number and the worker, whose turns live on the worker. The
-- worker's ID has no foreign key, since workers belong to another module and
-- the conversation outlives a removed worker; the name is the one the worker
-- had when the task started.
ALTER TABLE runtime_conversations ADD COLUMN task_worker_id TEXT;

ALTER TABLE runtime_conversations ADD COLUMN task_worker_name TEXT;
