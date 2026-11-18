-- A service's pointers into its channel's log: the ops addressed to it, by
-- hash, with what sorts them in the shared order. The op itself is read from
-- the channel's log.
CREATE TABLE IF NOT EXISTS carbon_pointer (
  hash TEXT PRIMARY KEY,
  time INTEGER NOT NULL,
  device TEXT
) WITHOUT ROWID;

-- Results: what this device did with each op addressed to this service, keyed
-- by the op's hash. An op is applied, with its output, or refused, with its
-- service error; either way with the calls it made, the tables and rows it
-- read, and its changeset, its undo record, whose tables are its write set. A
-- refused op rolled its writes back, so its changeset is empty. Results never
-- leave this device, and a rewind replaces them. The pointers with no result
-- yet are pending.
CREATE TABLE IF NOT EXISTS carbon_result (
  hash TEXT PRIMARY KEY,
  outcome TEXT NOT NULL CHECK (outcome IN ('applied', 'refused')),
  output TEXT,
  error TEXT,
  calls TEXT NOT NULL,
  read_set TEXT NOT NULL DEFAULT '[]',
  changeset BLOB NOT NULL DEFAULT x''
) WITHOUT ROWID;
