CREATE UNIQUE INDEX IF NOT EXISTS idx_travelers_auth_token
  ON travelers(auth_token) WHERE auth_token IS NOT NULL;
