-- Runs that a retry keeps (docs/features/system-tools/tool-calls.md, retry).
-- A retry replaces the reply of the run before it, unless that run made tool
-- calls: then its parts stay in the turn, for the owner and the model, and the
-- new run continues after them. Runs stored before this column existed were
-- replaced, so they read as not kept.
ALTER TABLE runtime_runs ADD COLUMN kept INTEGER NOT NULL DEFAULT 0 CHECK (kept IN (0, 1));
