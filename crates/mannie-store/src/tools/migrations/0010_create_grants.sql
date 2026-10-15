-- Grants: permissions that the owner gave for the rest of one conversation
-- (docs/features/system-tools/operations-and-approvals.md, grants). A grant
-- belongs to a conversation of the runtime's table, so the foreign key crosses
-- modules on purpose: deleting a conversation deletes its grants.
CREATE TABLE tools_grants (
  id TEXT PRIMARY KEY NOT NULL,
  conversation_id TEXT NOT NULL REFERENCES runtime_conversations (id) ON DELETE CASCADE,
  -- What the grant covers, as a JSON document of the version in `version`.
  version INTEGER NOT NULL CHECK (version > 0),
  scope TEXT NOT NULL CHECK (json_valid(scope)),
  created_at INTEGER NOT NULL
) STRICT;

CREATE INDEX tools_grants_by_conversation ON tools_grants (conversation_id, created_at);
