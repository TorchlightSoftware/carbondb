-- The system database: the identities on this device, each with its first
-- device's secret key, which no space ever holds.
CREATE TABLE IF NOT EXISTS carbon_system_identity (
  slug TEXT PRIMARY KEY,
  id TEXT NOT NULL UNIQUE,
  space TEXT NOT NULL,
  personal TEXT NOT NULL,
  device TEXT NOT NULL,
  secret TEXT NOT NULL
);
