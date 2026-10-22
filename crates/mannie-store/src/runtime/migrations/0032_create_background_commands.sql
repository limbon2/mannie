-- Background commands (ADR-0041, docs/features/system-tools/
-- background-commands.md): commands that run past the tool call that started
-- them. A command's record outlives its call's part, which stays final, and
-- goes with its conversation.
CREATE TABLE runtime_background_commands (
  conversation_id TEXT NOT NULL REFERENCES runtime_conversations (id) ON DELETE CASCADE,
  -- Its number in the conversation, from 1. A conversation never reuses one.
  number INTEGER NOT NULL CHECK (number > 0),
  -- The part of the call that started it, under which its output is
  -- archived when it ends.
  part_id TEXT NOT NULL REFERENCES runtime_parts (id) ON DELETE CASCADE,
  -- `local`, or the ID of the plugin whose tool runs it on a remote host.
  source TEXT NOT NULL,
  -- Whether it runs, so the recovery after a crash finds it.
  running INTEGER NOT NULL CHECK (running IN (0, 1)),
  -- The rest of the record, as a JSON document of the version in `version`.
  version INTEGER NOT NULL CHECK (version > 0),
  document TEXT NOT NULL CHECK (json_valid(document)),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, number)
) STRICT;

-- The recovery reads the commands that ran when the process ended.
CREATE INDEX runtime_background_commands_running ON runtime_background_commands (running) WHERE running = 1;
