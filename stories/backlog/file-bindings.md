# A service can store files in a space

Status: backlog

## Idea

Services keep more than rows: retained documents, images, exports. A `file` binding in `service.toml` gives a service object storage in the space, the way `sql` gives it a database. The `wrangler.toml` generator translates `r2_buckets` to it. Each service's file store is its own, per space, like its `sql` files (see [packages](../packages.md)).

## To decide

- How files replicate: inside ops, as blobs p2panda carries, or by hash with the bytes fetched separately.
- Whether a file write is part of the op's transaction, so a rewind removes it.
- Size limits, and what a device does while a file it needs hasn't arrived yet.
- The handle a handler gets: put, get, list and delete by key.
