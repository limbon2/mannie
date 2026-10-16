-- The name that the owner gave a worker in Mannie (docs/features/workers/
-- worker-activity.md, naming a worker). NULL means the name in the worker's
-- configuration, which `name` keeps and every `Hello` updates.
ALTER TABLE workers_paired ADD COLUMN chosen_name TEXT;
