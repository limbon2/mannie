-- Shared grants: permissions that the owner gave for every conversation of a
-- project, or for every conversation (docs/features/system-tools/
-- operations-and-approvals.md, grants). Removing a project removes its
-- grants; the foreign key crosses modules on purpose.
CREATE TABLE tools_shared_grants (
  id TEXT PRIMARY KEY NOT NULL,
  -- The project, or NULL for every conversation.
  project_id TEXT REFERENCES runtime_projects (id) ON DELETE CASCADE,
  -- What the grant covers, as a JSON document of the version in `version`.
  version INTEGER NOT NULL CHECK (version > 0),
  scope TEXT NOT NULL CHECK (json_valid(scope)),
  created_at INTEGER NOT NULL
) STRICT;

-- The same grant is saved once for the same project or for everyone.
CREATE UNIQUE INDEX tools_shared_grants_unique ON tools_shared_grants (COALESCE(project_id, ''), scope);
