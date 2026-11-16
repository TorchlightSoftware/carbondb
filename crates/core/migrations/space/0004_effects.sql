-- An op's effects: what its first run did outside its service's own tables
-- (a call to another service, a space created), each with its name, input and
-- output, success or failure, in the order made. Written onto its row once its
-- first run settles; every later run replays these outputs. The op's hash does
-- not cover them, since they are not part of its call.
ALTER TABLE carbon_oplog RENAME COLUMN sub_calls TO effects;
