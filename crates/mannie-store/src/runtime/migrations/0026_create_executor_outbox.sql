-- The deletions of task conversations that a plugin's executor serves, in
-- the transaction that deletes their parent, until the executor takes them
-- (ADR-0031, decision 9). An executor takes them while its plugin is on, and
-- when the plugin starts again after it was off.
CREATE TABLE runtime_executor_outbox (
  executor_kind TEXT NOT NULL,
  executor_target TEXT NOT NULL,
  conversation_id TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (executor_kind, executor_target, conversation_id)
) STRICT, WITHOUT ROWID;
