-- [device]: the device keys an identity space authorizes, and the names and
-- tags a personal space gives them.
CREATE TABLE carbon_device (
  pubkey TEXT PRIMARY KEY
);
CREATE TABLE carbon_device_ref (
  pubkey TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  tags TEXT NOT NULL
);
