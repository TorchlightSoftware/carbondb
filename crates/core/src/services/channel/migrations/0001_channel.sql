-- [channel]: what the channel is, fixed by its first op, and whether it is
-- sealed; the services it names; the channels it points to; its members and
-- the invitations not yet accepted.
CREATE TABLE origin (
  one INTEGER PRIMARY KEY CHECK (one = 1),
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  manage TEXT NOT NULL,
  sealed INTEGER NOT NULL DEFAULT 0
) WITHOUT ROWID;
CREATE TABLE service (
  name TEXT PRIMARY KEY,
  space TEXT NOT NULL,
  package TEXT NOT NULL,
  version TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE ref (
  channel TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  slug TEXT NOT NULL,
  cascade TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE member (
  identity TEXT PRIMARY KEY,
  access TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE invitation (
  invitation TEXT PRIMARY KEY,
  contact TEXT NOT NULL,
  access TEXT NOT NULL,
  invited_by TEXT NOT NULL
) WITHOUT ROWID;
