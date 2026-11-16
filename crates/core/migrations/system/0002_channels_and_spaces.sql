-- An identity's channels: its own, running [identity], and its personal one.
ALTER TABLE carbon_system_identity RENAME COLUMN space TO channel;
-- The spaces this device holds, each running one service: the package
-- registered here that runs it, the name its channel calls it by, and the
-- channel it belongs to (a channel's own space belongs to itself). This
-- device's own bookkeeping, never replicated.
CREATE TABLE carbon_system_space (
  id TEXT PRIMARY KEY,
  package TEXT NOT NULL,
  name TEXT NOT NULL,
  channel TEXT
) WITHOUT ROWID;
