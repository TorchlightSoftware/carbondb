# Reading the past: changesets, not a kept WAL

This is for anyone changing how CarbonDB reads a service's rows as they stood earlier, or tempted to bring back SQLite's WAL snapshots for it. It records what we chose, the measurements behind the choice, and the workloads that would make us look again.

**The decision:** a read through `callsync` rebuilds each row it reads from the changesets every store already keeps for rewinds, and runs its statement against that row on a scratch connection in memory. SQLite checkpoints each store's WAL as usual, so the data lives in two places on each device, the channel's op log and each service's tables, plus the changesets.

## What a read of the past needs

A read through `callsync`, made inside an op, answers as of that op's place in the shared order ([replication.md](replication.md#consistent-reads)). A store applies ops in the order they reach it, so no single commit need hold every row as it stood at a place. One row's history is in order, though, so each statement looks up one row by its whole key, and that row is read as it stood. Two ways can give that row:

- **A kept WAL.** Each op records the SQLite snapshot of the commit it started from, and the store never checkpoints, so every commit stays readable (`sqlite3_snapshot_open`). The row is read at the snapshot of the first op after the reader's place that wrote it.
- **Changesets.** Each op's result keeps its changeset, which rewinds invert. A changeset holds a deleted row whole, and for an update the old values of the columns it changed, so undoing, newest first, the changes the ops after the reader's place made to the row gives the row as it stood.

We built the kept WAL first, as the built-in primitive, then measured both.

## What we measured

`spikes/changeset-reads` keeps a store as the core does: 10,000 ops, each writing one busy row and one other row in its own transaction, recording its place, changeset, start snapshot and written rows. Measured 2026-11-20 on one laptop (Intel i7-7700HQ); the spike's [README](../spikes/changeset-reads/README.md) has every number and how to run it again.

| Kept on disk | Per op |
|---|---|
| WAL, never checkpointed | 43,655 bytes |
| Changesets | 84 bytes |
| Start snapshots | 48 bytes |

The WAL keeps whole 4 KB pages: the table's, the core's record of the op, and the B-tree pages above them. At one op a second, 48 hours of kept WAL is about 7.5 GB per service; the changesets for the same ops are about 15 MB, and rewinds need them anyway.

Median time to read one row as of a place, by how many ops placed after the reader wrote that row, in µs:

| Later writes | Kept WAL | Rebuilt |
|---|---|---|
| 0 | 21 | 12 |
| 1 | 53 | 17 |
| 10 | 54 | 37 |
| 100 | 54 | 253 |
| 1,000 | 54 | 2,437 |
| 10,000 | 77 | 24,335 |

A rebuild costs about 2.4 µs per later write to the row; the kept WAL costs the same at any depth. They break even near twenty later writes to the one row. How far behind the reader is overall does not matter, only how often that row changed since.

## Where each would win

Rebuilding wins:

- **Roles and membership**, the main thing `callsync` reads. They change rarely, so even a device two days behind sees a few later writes to the row it reads.
- **Every op, always.** The kept WAL costs 44 KB on every write in every service, whether anyone reads the past or not; rebuilding costs nothing until a late read.
- **Many services in a channel.** Each store keeps its own WAL, so its cost multiplies by the number of services.
- **Phones.** A busy chat at 50,000 messages a day would keep about 2 GB of WAL a day per service.

The kept WAL wins:

- **A busy row read from far back,** such as an inventory count every sale writes and checks: a device back after two days with 10,000 sales since reads it in 24 ms rebuilt, against 77 µs.
- **Many readers of that row re-run at once.** When a late write lands before a thousand readers, each rebuilds back to its own place, so the work grows with the square of their number: about a second, against 50 ms.

Those workloads are slow already: a late op that writes such a row rewinds every later op that touched it, and each re-run handler costs more than a rebuild step. A service that keeps shared rows from being written by every op avoids both costs.

If busy rows read from far back come to matter, two remedies stay within our own tables: keep each row's old values beside its entry in `carbon_write`, so a rebuild step skips parsing the op's whole changeset; or keep a whole copy of a row every so many writes, so no rebuild walks back further than that.

## Running the statement against the rebuilt row

The rebuilt row is in no table of the store, so the reader's statement runs unchanged on a scratch connection in memory, kept per store (`crates/core/src/store/scratch.rs`). Each table there is declared from the store's own `CREATE TABLE` text, holds only the row being read, and is declared again when that text changes. The connection carries the same authorizer and functions as any reader, and the statement must still read only its one row.

- **Rejected: a `WITH` clause named for the table.** The cheapest, but its columns have no type affinity or collation, so a statement answers differently: `name = 'ANN'` stops matching `Ann` under `COLLATE NOCASE`.
- **Rejected: a temporary table on the reader's connection.** It answers correctly, but creating and dropping it costs about 90 µs per read; the scratch connection costs about 5 µs.

A column the table gained since the reader's place keeps its current value, or its default for a row deleted since, so the service's current code reads the columns it expects. Generated columns are left to SQLite, as no changeset holds them.

## Finding the writes after a place

`carbon_write` and `carbon_read` are keyed by table, key and place, so the writes or reads of a row after a place are one range of the index. Keyed by table, key and op hash, as first built, finding them read every write the row ever had: about 4 ms per read of a row with 10,000 writes, whichever way it was then read.

## What this leaves open

- **Pruning.** Changesets and the op log grow until a horizon lets a device drop what is older than its cutoff; that is decided separately ([old ops](../stories/backlog/old-ops.md)).
- **Measuring on phones.** The numbers above are from one laptop.
