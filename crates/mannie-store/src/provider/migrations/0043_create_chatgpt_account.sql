-- The ChatGPT account through which profiles use the owner's ChatGPT plan
-- (ADR-0062). One row at most: Mannie holds one account. Its tokens live in
-- the operating system's credential store, split into parts; this row holds
-- only which generation of parts is current and how many there are.
--
-- The row is created by the first sign-in and never deleted, so that the
-- installation's host ID and the client that OpenAI issued survive a sign-out.
CREATE TABLE provider_chatgpt_account (
  id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
  -- The installation's ID for OpenAI's `ext_agent_host_id`: `urn:uuid:` and a
  -- random UUID of version 4. Not a secret.
  host_id TEXT NOT NULL CHECK (length(host_id) BETWEEN 1 AND 100),
  -- The client that OpenAI issued at the first sign-in, which later sign-ins
  -- reuse. NULL until a sign-in completed the registration.
  client_id TEXT CHECK (client_id IS NULL OR length(client_id) BETWEEN 1 AND 200),
  -- The `sub` of the account's ID token, which tells accounts apart.
  subject TEXT CHECK (subject IS NULL OR length(subject) BETWEEN 1 AND 500),
  -- The account's email address, which the settings show.
  email TEXT CHECK (email IS NULL OR length(email) BETWEEN 1 AND 500),
  state TEXT NOT NULL CHECK (state IN ('signed_out', 'signed_in', 'sign_in_needed')),
  -- The generation of the token parts in the credential store, which each
  -- save of new tokens increases, and how many parts it has.
  token_generation INTEGER NOT NULL DEFAULT 0 CHECK (token_generation >= 0),
  token_parts INTEGER NOT NULL DEFAULT 0 CHECK (token_parts BETWEEN 0 AND 32),
  -- Whether the owner saw the notice that Mannie uses their ChatGPT plan.
  notice_seen INTEGER NOT NULL DEFAULT 0 CHECK (notice_seen IN (0, 1)),
  updated_at INTEGER NOT NULL,
  CHECK ((state = 'signed_in') = (token_parts > 0)),
  CHECK (state = 'signed_out' OR (client_id IS NOT NULL AND subject IS NOT NULL))
) STRICT;
