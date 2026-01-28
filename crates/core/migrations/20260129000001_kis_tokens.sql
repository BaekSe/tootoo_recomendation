-- Store KIS access tokens to reduce re-issuance across runs.
-- NOTE: This stores a bearer token. Keep DB access restricted.

CREATE TABLE IF NOT EXISTS kis_access_tokens (
  env text PRIMARY KEY,
  access_token text NOT NULL,
  access_token_token_expired text,
  expires_in bigint,
  issued_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS kis_access_tokens_updated_at_idx
  ON kis_access_tokens (updated_at);
