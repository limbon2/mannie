-- The executor of a task that a plugin runs (ADR-0031, decision 7): its
-- kind, its target, the version of the latest state that it published, and
-- a document of its own that the executor reads and writes and the core
-- keeps as it is. A task that 0.2.0 delegated to a worker becomes a task of
-- the Workers plugin's executor, whose document keeps what the worker's
-- columns held. The task's own state, ending and times stay in their columns.
--
-- The table's check still asks a task without a turn for a worker, so the
-- core keeps writing an executor's target into worker_id until a later
-- release builds the table again without the worker's columns.
ALTER TABLE runtime_tasks ADD COLUMN executor_kind TEXT;

ALTER TABLE runtime_tasks ADD COLUMN executor_target TEXT;

ALTER TABLE runtime_tasks ADD COLUMN executor_version INTEGER NOT NULL DEFAULT 0 CHECK (executor_version >= 0);

ALTER TABLE runtime_tasks ADD COLUMN executor_document TEXT;

UPDATE runtime_tasks
SET
  executor_kind = 'workers',
  executor_target = worker_id,
  executor_version = coalesce(remote_version, 0),
  executor_document = json_object(
    'version', 1,
    'profileId', profile_id,
    'submission', submission,
    'cancelRequestedAt', cancel_requested_at
  )
WHERE worker_id IS NOT NULL;

CREATE INDEX runtime_tasks_by_executor ON runtime_tasks (executor_kind, state)
WHERE executor_kind IS NOT NULL;
