# session-rowid spike

Can SQLite sessions record tables without a primary key, and is that safe for rewind? SQLite can, and rusqlite does not expose it, but the question turned out to be the wrong one: any key SQLite assigns is unsafe, because it can move under the row's author. A row's key must come from the op that creates it, which the core now checks after every service's `init`.

```
cargo test
```

## What it found

- **The bundled SQLite supports it.** rusqlite 0.39.0 bundles SQLite 3.51.3 through libsqlite3-sys 0.37.0. `sqlite3session_object_config` with `SQLITE_SESSION_OBJCONFIG_ROWID` makes a session record a table without a primary key as if it had `_rowid_ INTEGER PRIMARY KEY` as its first column. It must be set before the first table is attached.
- **rusqlite does not expose it.** Its `Session` keeps the raw handle private and has no `object_config`. The spike creates the session through `rusqlite::ffi` and hands the changeset to rusqlite's stream functions (`invert_strm`, `apply_strm`) as bytes.
- **By default, a table without a primary key is ignored:** its changes never reach the changeset, so a rewind would leave them in place.
- **With the option, a keyless table rewinds to the same final state,** and a replica that rewinds ends with the same rowids as one that applied the same ops in order, because SQLite assigns a new rowid as one past the largest, and inverting a changeset restores the table exactly.
- **`AUTOINCREMENT` keys drift after a rewind, primary key or not.** The counter in `sqlite_sequence` is not part of the changeset, so inverting an insert does not wind it back, and the re-run op gets a higher id than on a replica that never rewound.
- **`VACUUM` renumbers the rowids of a table without an explicit `INTEGER PRIMARY KEY`.** A changeset kept for a later rewind names rows by rowid, so it would then point at the wrong rows.
- **An assigned key moves under its author after a rewind.** The author of note b saw id 2; once an earlier op c arrives and b re-runs, b is 3 and c is 2, so a later op naming note 2 edits c. Final states match across replicas, but the reference is wrong. This holds for a keyless rowid and for an `INTEGER PRIMARY KEY` left out of the insert alike.
- **A `WITHOUT ROWID` table is recorded by sessions, refuses an insert without its key, and cannot be declared `AUTOINCREMENT`.**
- **A savepoint rolled back inside the transaction leaves nothing in the changeset,** so a nested call that fails and rolls back alone neither reaches a rewind nor counts as written.
- **A changeset holds only net changes:** an update matching no rows, an update to the same values, and a row inserted then deleted record nothing. This is why the core takes an op's write set from its changeset.
- **A `WITHOUT ROWID` key's `DEFAULT` can call a SQL function the host registers,** drawn per row, for multi-row inserts and `INSERT … SELECT` too. In an ordinary table an `INTEGER PRIMARY KEY` is the rowid, and SQLite assigns it before any default applies. This is what `op_id()` in the core uses.
