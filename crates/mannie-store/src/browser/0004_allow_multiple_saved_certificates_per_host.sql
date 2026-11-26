-- Allow saving more than one trusted certificate fingerprint per hostname,
-- such as separate HTTPS services on different ports of one host (ADR-0082).
CREATE TABLE browser_saved_certificates_next (
  hostname TEXT NOT NULL,
  fingerprint BLOB NOT NULL CHECK (length(fingerprint) = 32),
  PRIMARY KEY (hostname, fingerprint)
) STRICT, WITHOUT ROWID;

INSERT INTO browser_saved_certificates_next (hostname, fingerprint)
SELECT hostname, fingerprint FROM browser_saved_certificates;

DROP TABLE browser_saved_certificates;
ALTER TABLE browser_saved_certificates_next RENAME TO browser_saved_certificates;
