-- [chat]: messages in a space. A message's id comes from the op that posts it.
CREATE TABLE message (
  id TEXT PRIMARY KEY,
  author TEXT NOT NULL,
  text TEXT NOT NULL,
  sent_at INTEGER NOT NULL,
  edited_at INTEGER,
  deleted_at INTEGER
) WITHOUT ROWID;
