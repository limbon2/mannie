-- The switches of skills (ADR-0061, decision 5): the owner's choice of whether
-- agents see a skill, for this computer's skills (project_id NULL) or a
-- project's. A skill without a row keeps its place's default: on for this
-- computer, off for a project. Removing a project removes its switches, so
-- adding the folder again starts with its skills off.
CREATE TABLE runtime_skill_switches (
  project_id TEXT REFERENCES runtime_projects (id) ON DELETE CASCADE,
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 64),
  is_on INTEGER NOT NULL CHECK (is_on IN (0, 1)),
  -- For a project's skill that the owner turned on: the digest of what agents'
  -- list showed of it, so a change turns it off again. NULL otherwise.
  digest TEXT CHECK (digest IS NULL OR length(digest) = 64),
  updated_at INTEGER NOT NULL
) STRICT;

-- One switch for each skill of a place.
CREATE UNIQUE INDEX runtime_skill_switches_unique ON runtime_skill_switches (COALESCE(project_id, ''), name);
