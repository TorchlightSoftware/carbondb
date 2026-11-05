-- [space]: what the space is, fixed by its first op, and its refs to other spaces.
CREATE TABLE carbon_origin (
  one INTEGER PRIMARY KEY CHECK (one = 1),
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  services TEXT NOT NULL,
  manage TEXT NOT NULL
);
CREATE TABLE carbon_ref (
  space TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  slug TEXT NOT NULL,
  cascade TEXT NOT NULL,
  UNIQUE (kind, slug)
);
