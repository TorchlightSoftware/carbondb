-- What each op's transaction did on this device, kept with its result and
-- replaced with it when a rewind re-runs the op: its read set, the tables its
-- statements read, from SQLite's authorizer, as a JSON array; and its
-- changeset, the rows it changed with their old and new values, from a SQLite
-- session: its undo record, and the tables it holds are its write set. A
-- refused op rolled its writes back, so its changeset is empty, but its read
-- set stands. Neither travels with the op: every replica works out its own.
-- Results from before this migration read as reading and writing nothing.
ALTER TABLE carbon_result ADD COLUMN read_set TEXT NOT NULL DEFAULT '[]';
ALTER TABLE carbon_result ADD COLUMN changeset BLOB NOT NULL DEFAULT x'';
