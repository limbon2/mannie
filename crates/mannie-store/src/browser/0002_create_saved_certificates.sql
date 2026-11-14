-- Saved trust is independent of Browser sessions and their confirmation journal.
CREATE TABLE browser_saved_certificates (
  hostname TEXT PRIMARY KEY NOT NULL,
  fingerprint BLOB NOT NULL CHECK (length(fingerprint) = 32)
) STRICT, WITHOUT ROWID;

-- A save or removal and its journal entry are committed together.
CREATE TABLE browser_certificate_trust_changes (
  id INTEGER PRIMARY KEY,
  hostname TEXT NOT NULL,
  fingerprint BLOB NOT NULL CHECK (length(fingerprint) = 32),
  action TEXT NOT NULL CHECK (action IN ('save', 'forget')),
  recorded_at INTEGER NOT NULL
) STRICT;
