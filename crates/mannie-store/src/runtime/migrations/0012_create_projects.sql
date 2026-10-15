-- Projects: folders that the owner added, whose conversations work in them
-- (docs/features/chat/projects.md, ADR-0023). The folder is stored with its
-- links resolved, as the runtime checked it, and each folder is one project.
CREATE TABLE runtime_projects (
  id TEXT PRIMARY KEY NOT NULL,
  -- The name the owner gave the project, or NULL for the folder's name.
  name TEXT,
  folder TEXT NOT NULL UNIQUE,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
) STRICT;
