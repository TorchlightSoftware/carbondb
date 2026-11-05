-- The op log: every op applied here, in the order applied, appended once and
-- never changed. In a space it holds replicated ops, and ops on local services
-- marked with the device that ran them; in the system database, the ops on
-- local services made on the system handle. The output and the calls an op
-- made are this device's own record, never sent with the op.
CREATE TABLE IF NOT EXISTS carbon_oplog (
  hash TEXT PRIMARY KEY,
  time INTEGER NOT NULL,
  device TEXT,
  author TEXT,
  key TEXT NOT NULL,
  input TEXT NOT NULL,
  version INTEGER NOT NULL,
  output TEXT NOT NULL,
  calls TEXT NOT NULL
);
