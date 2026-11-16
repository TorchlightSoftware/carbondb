# A service can store files

Status: backlog

## Idea

Services keep more than rows: retained documents, images, exports. A `file` binding in `service.toml` gives a service object storage, the way `sql` gives it a database. The `wrangler.toml` generator translates `r2_buckets` to it. Each instance of a service has its own file store, as it has its own SQLite file (see [packages](../packages.md)). Unlike `fetch`, which reaches the world outside and is recorded as an effect, a file store is the service's own state, replicated with it.

## To decide

- How files replicate: inside ops, as blobs p2panda carries, or by hash with the bytes fetched separately.
- Whether a file write is part of the op's transaction, so a rewind removes it.
- Size limits, and what a device does while a file it needs hasn't arrived yet.
- The handle a handler gets: put, get, list and delete by key.
