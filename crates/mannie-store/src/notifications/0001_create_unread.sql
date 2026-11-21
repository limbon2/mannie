-- Unread finished conversations for desktop notifications and the tray badge
-- (ADR-0074). A row is written when an owner conversation finishes or fails
-- while not viewed in a focused window, and removed when opened, started
-- again or deleted.
CREATE TABLE notifications_unread (
  conversation_id TEXT PRIMARY KEY NOT NULL,
  marked_at INTEGER NOT NULL
) STRICT, WITHOUT ROWID;
