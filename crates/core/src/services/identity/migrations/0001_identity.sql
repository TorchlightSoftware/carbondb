-- [identity]: the identity, named by its first device's key, and the devices
-- that act for it, each with its name and tags.
CREATE TABLE identity (
  id TEXT PRIMARY KEY,
  device TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE device (
  pubkey TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  tags TEXT NOT NULL
) WITHOUT ROWID;
