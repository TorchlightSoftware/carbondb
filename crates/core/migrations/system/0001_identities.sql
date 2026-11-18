-- The system: the identities on this device, each with its own channel, its
-- personal channel, this device's key in it, and that key's secret, which no
-- channel ever holds.
CREATE TABLE IF NOT EXISTS carbon_system_identity (
  slug TEXT PRIMARY KEY,
  id TEXT NOT NULL UNIQUE,
  channel TEXT NOT NULL,
  personal TEXT NOT NULL,
  device TEXT NOT NULL,
  secret TEXT NOT NULL
);
