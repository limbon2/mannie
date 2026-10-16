-- The deletions of task conversations that waited for a worker in 0.2.0 move
-- into the core's outbox, which carries every deletion that a plugin's
-- executor delivers from now on (ADR-0031, decision 9). The core's modules
-- migrate before a plugin's, so the outbox exists here. The table stays
-- empty and unused until the release after drops it (ADR-0002).
INSERT OR IGNORE INTO runtime_executor_outbox (
  executor_kind, executor_target, conversation_id, created_at
)
SELECT 'workers', worker_id, conversation_id, created_at
FROM workers_deletions;

DELETE FROM workers_deletions;
