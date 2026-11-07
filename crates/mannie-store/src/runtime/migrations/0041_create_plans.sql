-- Plans (ADR-0060): what an agent saved for the owner to read, edit and run
-- later. A conversation holds one plan, and a plan outlives its conversation
-- and its project. The number never returns after a delete, so an old mention
-- of a deleted plan never names another one.
CREATE TABLE runtime_plans (
  number INTEGER PRIMARY KEY AUTOINCREMENT,
  conversation_id TEXT UNIQUE REFERENCES runtime_conversations (id) ON DELETE SET NULL,
  project_id TEXT REFERENCES runtime_projects (id) ON DELETE SET NULL,
  title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 80),
  body TEXT NOT NULL CHECK (length(body) >= 1),
  -- Whether the owner edited it since the model last saved it, so the model's
  -- next save reads the owner's version first.
  owner_edited INTEGER NOT NULL DEFAULT 0 CHECK (owner_edited IN (0, 1)),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
) STRICT;
