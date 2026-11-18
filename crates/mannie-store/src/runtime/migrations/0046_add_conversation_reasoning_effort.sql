-- An absent choice follows the profile, including on conversations from older versions.
ALTER TABLE runtime_conversations ADD COLUMN reasoning_effort TEXT;

CREATE TABLE runtime_reasoning_preference (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    reasoning_effort TEXT
) STRICT;
