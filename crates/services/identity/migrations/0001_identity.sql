-- [identity]: the identity whose identity space this is, and its first device.
CREATE TABLE identity (
  id TEXT PRIMARY KEY,
  device TEXT NOT NULL
) WITHOUT ROWID;
