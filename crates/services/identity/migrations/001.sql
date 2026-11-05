-- [identity]: the identity whose identity space this is, and its first device.
CREATE TABLE carbon_identity (
  id TEXT PRIMARY KEY,
  device TEXT NOT NULL
);
