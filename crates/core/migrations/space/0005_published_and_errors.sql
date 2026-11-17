-- What this device published of its own ops, by op hash: the op log is the
-- outbox, and a rewind, which can make an op publishable late and out of log
-- order, forgets results but leaves this alone.
CREATE TABLE IF NOT EXISTS carbon_published (
  hash TEXT PRIMARY KEY
) WITHOUT ROWID;

-- Messages that failed the check every received op passes, kept as evidence
-- and never applied: keyed by their p2panda operation hash, since the op hash
-- they claim is unproven, with the signed operation, its p2panda author, when
-- it arrived and which check it failed.
CREATE TABLE IF NOT EXISTS carbon_error (
  operation TEXT PRIMARY KEY,
  author TEXT NOT NULL,
  arrived INTEGER NOT NULL,
  failed TEXT NOT NULL,
  signed TEXT NOT NULL
) WITHOUT ROWID;
