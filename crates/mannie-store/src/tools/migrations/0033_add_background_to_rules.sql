-- Rules for commands in the background (ADR-0041, D40): a rule saved from the
-- approval of a command in the background also covers commands in the
-- background, and a rule saved before it existed covers only the foreground.
ALTER TABLE tools_rules ADD COLUMN background INTEGER NOT NULL DEFAULT 0 CHECK (background IN (0, 1));

ALTER TABLE tools_remote_rules ADD COLUMN background INTEGER NOT NULL DEFAULT 0 CHECK (background IN (0, 1));
