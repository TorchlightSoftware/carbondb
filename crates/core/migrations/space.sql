-- What the core keeps in every space for itself: the services provisioned
-- there, and how many of each one's migrations have run.
CREATE TABLE IF NOT EXISTS carbon_migration (
  service TEXT PRIMARY KEY,
  count INTEGER NOT NULL
);
