-- The owner confirmed a certificate for an isolated Browser session.
-- The engine never reads this journal as a source of trust.
CREATE TABLE browser_certificate_confirmations (
  conversation_id TEXT NOT NULL REFERENCES runtime_conversations (id) ON DELETE CASCADE,
  session_id TEXT NOT NULL,
  hostname TEXT NOT NULL,
  fingerprint BLOB NOT NULL CHECK (length(fingerprint) = 32),
  confirmed_at INTEGER NOT NULL,
  PRIMARY KEY (session_id, hostname, fingerprint)
) STRICT, WITHOUT ROWID;

CREATE INDEX browser_certificate_confirmations_by_conversation
  ON browser_certificate_confirmations (conversation_id);
