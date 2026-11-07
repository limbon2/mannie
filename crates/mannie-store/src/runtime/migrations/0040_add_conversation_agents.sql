-- The agent of each of the owner's conversations (ADR-0059): NULL for the
-- built-in one, as every conversation from before custom agents is. A deleted
-- agent's key reads as the built-in one too, so the column refers to nothing.
ALTER TABLE runtime_conversations ADD COLUMN agent_key TEXT
  CHECK (agent_key IS NULL OR length(agent_key) BETWEEN 1 AND 128);
CREATE INDEX runtime_conversations_by_agent
  ON runtime_conversations (agent_key)
  WHERE agent_key IS NOT NULL;
