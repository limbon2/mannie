-- The attachments of conversations (ADR-0046, decision 1): keys of context
-- providers, such as notebooks, with the labels that the providers gave them.
-- An attachment goes with its conversation, so a plugin never needs to hear
-- of a deletion, and stays while its plugin is off.
CREATE TABLE runtime_conversation_attachments (
  conversation_id TEXT NOT NULL REFERENCES runtime_conversations (id) ON DELETE CASCADE,
  -- The context provider's ID.
  provider TEXT NOT NULL,
  -- The provider's key, which it checked when the owner attached it.
  attachment_key TEXT NOT NULL CHECK (length(attachment_key) BETWEEN 1 AND 256),
  -- The order in which the owner attached them, from 0.
  position INTEGER NOT NULL CHECK (position >= 0),
  -- What the owner reads, as the provider gave it.
  label TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, provider, attachment_key),
  UNIQUE (conversation_id, position)
) STRICT;
