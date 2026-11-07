-- A local op's sub-calls: the calls it made to replicated services in its
-- space whose writes committed, each stamped as an op, in the order made, and
-- written onto its row when it commits. Other devices, and rewind here, replay
-- these, never the local op itself. The op's hash does not cover them, since
-- they are not part of its call. Every other op leaves this empty.
ALTER TABLE carbon_oplog ADD COLUMN sub_calls TEXT;
