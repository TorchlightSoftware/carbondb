-- The rows each applied op wrote, by table and key, from its changeset, with
-- the op's place in the shared order (a device of '' for none): a read of a
-- row as of a place finds the ops placed after it that wrote it as one range
-- of this key, and undoes their changes to it, newest first.
CREATE TABLE IF NOT EXISTS carbon_write (
  tbl TEXT NOT NULL,
  key TEXT NOT NULL,
  time INTEGER NOT NULL,
  device TEXT NOT NULL,
  hash TEXT NOT NULL,
  PRIMARY KEY (tbl, key, time, device, hash)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS carbon_write_by_op ON carbon_write (hash);

-- The rows of this service that ops of other services in its channel read
-- through `callsync`: the reader op, the store it applied in, and its place in
-- the shared order (a device of '' for none). An op applied here at an earlier
-- place that writes one of these rows, or undone here, re-runs the readers
-- placed after it, one range of this key. A reader undone in its own store
-- leaves its record behind until then, costing a re-run at most.
CREATE TABLE IF NOT EXISTS carbon_read (
  tbl TEXT NOT NULL,
  key TEXT NOT NULL,
  time INTEGER NOT NULL,
  device TEXT NOT NULL,
  reader TEXT NOT NULL,
  store TEXT NOT NULL,
  PRIMARY KEY (tbl, key, time, device, reader)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS carbon_read_by_reader ON carbon_read (reader);
