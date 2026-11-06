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
