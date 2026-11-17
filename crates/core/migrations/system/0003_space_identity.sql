-- The identity whose p2panda node syncs each space this device holds; none
-- for a space held single-node.
ALTER TABLE carbon_system_space ADD COLUMN identity TEXT;
