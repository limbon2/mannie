CREATE TABLE browser_site_credentials (
    reference TEXT PRIMARY KEY NOT NULL,
    origin TEXT NOT NULL,
    label TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending', 'active', 'revoked'))
) STRICT;

CREATE UNIQUE INDEX browser_site_credentials_label
ON browser_site_credentials (origin, label)
WHERE state <> 'revoked';

CREATE TABLE browser_site_credential_changes (
    id INTEGER PRIMARY KEY,
    reference TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('add', 'saved', 'remove', 'purged')),
    recorded_at INTEGER NOT NULL
) STRICT;
