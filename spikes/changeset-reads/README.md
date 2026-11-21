# changeset-reads spike

The core now reads this way; the decision and these numbers are in [docs/wal-vs-changesets.md](../../docs/wal-vs-changesets.md).

Can a `callsync` read rebuild a row as it stood at a place from the changesets a store already keeps for rewinds, instead of reading a WAL kept without checkpoints? Yes. The rebuilt row matches the kept WAL at every place, the WAL costs about 500 times the space, and a rebuild is faster than the kept WAL until about twenty later ops have written the row. Past that it costs about 2.4 µs per later write: a quarter of a millisecond at 100 later writes, 24 ms at 10,000.

```
cargo test                                   # whether a rebuilt row is the row as it stood, and how a statement reads it
cargo run --release -- [ops] [members] [runs] # what each way costs, by default 10000 ops, 1000 members, 50 runs
```

## Result

Measured 2026-11-20 on one laptop (Intel i7-7700HQ), with `cargo run --release -- 10000 1000 200`. Each of 10,000 ops writes the busy row `m1` and one other row in turn, in its own transaction, recording its place, changeset, start snapshot and written rows as the core does.

| Kept | Bytes | Per op |
|---|---|---|
| WAL, never checkpointed | 436,600,552 | 43,655 |
| changesets | 842,306 | 84 |
| start snapshots | 480,048 | 48 |

Median µs per read of one row as of a place, running `SELECT role, joined + 1 FROM member WHERE id = ?`:

| Row | Later writes | Kept WAL | Rebuilt, temp table | Rebuilt, scratch | Rebuilt, CTE |
|---|---|---|---|---|---|
| m1 | 0 | 21 | 100 | 12 | 7 |
| m1 | 1 | 53 | 110 | 17 | 10 |
| m1 | 10 | 54 | 135 | 37 | 30 |
| m1 | 100 | 54 | 368 | 253 | 236 |
| m1 | 1000 | 54 | 2,562 | 2,437 | 2,415 |
| m1 | 10000 | 77 | 24,541 | 24,335 | 24,277 |
| m500 | 0 | 20 | 101 | 13 | 7 |

Reading the current row directly takes 3 µs. Every way gave the same answer for every read; `main` checks that before timing.

## What it found

- **A rebuilt row is the row as it stood.** Take the row as it is now and undo, newest first, the change each op placed after the reader made to it. An insert undoes to no row, a delete to the whole old row it holds, and an update to the old values of the columns it changed; a changeset holds nothing else for an update. Across an insert, two updates, a delete, a second insert and an update of two columns, the rebuilt row equals the kept WAL's at every place (`a_row_rebuilt_from_later_changesets_is_the_row_the_kept_wal_holds_at_each_place`).
- **The WAL costs about 500 times what the changesets do.** It keeps whole 4 KB pages: the table's, the core's pointer, result and write index, and the B-tree pages above them. A changeset keeps the values changed. This run's WAL holds 44 KB per op, more than the 34 KB measured earlier through the SQLite shell, because the core's write index and start snapshots add pages.
- **A rebuild costs about 2.4 µs per later write to the row, and the kept WAL costs the same at any depth.** A reader near the latest op rebuilds little: with up to ten later writes, a rebuild through a scratch connection takes 12–37 µs, against 20–54 µs for the kept WAL. A reader 10,000 writes to its row behind takes 24 ms rebuilt and 77 µs from the kept WAL. Each later write reads that op's whole changeset and parses it; keeping each row's old values beside its entry in the write index would skip the parse, which this spike does not measure.
- **The statement must run against a table declared as the store's, or it can answer differently.** A common table expression named for the table is the cheapest and needs no table, but its columns have no type affinity and no collation. `id = 5` finds the row whose text key is `'5'` in the store's table and not in the expression; `name = 'ANN'` matches `Ann` under `COLLATE NOCASE` only in the table (`a_statement_answers_against_a_rebuilt_row_as_against_the_store_only_in_a_table_declared_the_same`).
- **A temporary table works on the read-only reader, inside its read, and costs about 90 µs.** SQLite finds a temporary table before the store's table of the same name, and a temporary table lives outside the store's file, so the store's WAL does not grow (`a_read_only_reader_holds_a_temporary_table_inside_its_read_and_leaves_the_store_untouched`). Creating and dropping it per read is what costs: two schema changes, each re-preparing the reader's statements.
- **A scratch connection in memory, declared as the store's table and kept per table, costs about 5 µs.** It reads with the store's declaration, so its answers match. It is a separate connection, though: the core would have to install the call window's authorizer and functions on it, and analyse the statement there, as it does on a reader.
- **The core's write index makes every read of a busy row slow, at a snapshot or rebuilt.** `carbon_write` is keyed by table, key and op hash, so finding the writers after a place joins every writer of the row to `carbon_pointer` and filters by place. With the core's index, the first run of this spike took about 4 ms for every read of `m1`, which has 10,000 writers, at any depth and either way. Keeping each writer's place in `carbon_write`, keyed by table, key and place, makes the writers after a place one range of the index, and this spike does so. The core had the slow shape when this ran, for writers and readers alike; it now keys both by place.
- **Nothing here needs SQLite's snapshot API to rebuild.** Only the kept-WAL column uses it, through `rusqlite::ffi`, for comparison; it needs `LIBSQLITE3_FLAGS=-DSQLITE_ENABLE_SNAPSHOT`, which this spike's `.cargo/config.toml` sets.
