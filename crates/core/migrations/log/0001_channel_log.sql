-- A channel's log: every op this device received for the channel's services,
-- local calls and synced ops alike, written as it arrives, before it applies,
-- and never changed but that it gains the effects its first run made. Each op
-- names the service it is addressed to, and the op whose call made it, its
-- cause, when one service called another.
CREATE TABLE IF NOT EXISTS carbon_oplog (
  hash TEXT PRIMARY KEY,
  time INTEGER NOT NULL,
  device TEXT,
  author TEXT,
  key TEXT NOT NULL,
  input TEXT NOT NULL,
  version TEXT NOT NULL,
  effects TEXT,
  service TEXT NOT NULL,
  cause TEXT
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS carbon_oplog_cause ON carbon_oplog (cause);

-- What became of each op on this device, applied or refused, as its service
-- recorded it: what a cause's calls follow, and what decides whether an op is
-- published. Forgotten with the result when a rewind makes the op pending.
CREATE TABLE IF NOT EXISTS carbon_outcome (
  hash TEXT PRIMARY KEY,
  outcome TEXT NOT NULL CHECK (outcome IN ('applied', 'refused'))
) WITHOUT ROWID;

-- What this device published of its own ops, by op hash; a rewind, which can
-- make an op publishable late and out of log order, leaves this alone.
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
