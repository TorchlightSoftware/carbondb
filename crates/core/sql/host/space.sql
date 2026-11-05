-- What the core keeps in every space for itself: how many of each service's
-- migrations have run there.
CREATE TABLE IF NOT EXISTS carbon_migration (
  service TEXT PRIMARY KEY,
  count INTEGER NOT NULL
);
