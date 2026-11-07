-- The op log: every op this device received, local calls and synced ops alike,
-- written as it arrives, before it applies, and never changed. In a space it
-- holds replicated ops, and ops on local services marked with the device that
-- ran them; in the system database, the ops made on the system handle. What
-- this device did with each op is its result, kept apart.
CREATE TABLE IF NOT EXISTS carbon_oplog (
  hash TEXT PRIMARY KEY,
  time INTEGER NOT NULL,
  device TEXT,
  author TEXT,
  key TEXT NOT NULL,
  input TEXT NOT NULL,
  version TEXT NOT NULL
) WITHOUT ROWID;

-- Results: what this device did with each op in the op log, keyed by the op's
-- hash. An op is applied, with its output, or refused, with its service error;
-- either way with the calls it made. Results never leave this device, and a
-- rewind replaces them. The ops with no result yet are pending.
CREATE TABLE IF NOT EXISTS carbon_result (
  hash TEXT PRIMARY KEY,
  outcome TEXT NOT NULL CHECK (outcome IN ('applied', 'refused')),
  output TEXT,
  error TEXT,
  calls TEXT NOT NULL
) WITHOUT ROWID;
