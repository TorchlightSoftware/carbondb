# Replication: how every replica of a service reaches the same state

This is for anyone who needs to know how CarbonDB keeps the copies of a service on different devices identical: what decides the order ops apply in, what happens when an op arrives late, what is undone and re-run, what a handler does outside its own tables, and why.

Each service runs in a channel, in a space of its own: one SQLite file holding its tables, its op log and its results, so an op and its transaction always cover the same data. Services call each other by message; between services only delivery is guaranteed.

Rewind and replay are built and tested on a single device: a device receives rows from other devices through `Carbon::receive`, and applies them in the shared order. Carrying rows between devices over p2panda is not built yet; see [stories/p2panda-sync.md](../stories/p2panda-sync.md).

## The promise

Every replica of a service that holds the same ops ends in the same state, whatever order the ops arrived in. Each replica gets there by applying every op in one shared order and re-running the service's own handler, so business rules give the same answer everywhere, against the state at the op's place in that order.

## Terms

- **Op**: a service call that writes, stamped with its author, its device, its time, the version of the service that handles it, and a hash over all of these that names it on every replica.
- **Log row**: an op as the op log holds it and as it travels between devices: the op and, once its first run has settled, the effects it made.
- **Effect**: what a handler does outside its own service's tables, such as calling another service: made on the op's first run, its output recorded on the log row.
- **Result**: what this device did with an op: applied with its output, or refused with its error; with the op's read set and changeset. Results never leave the device.
- **Pending op**: an op in the log with no result yet.
- **Read set**: what an op's statements read: one row, when a statement looks it up by its whole primary key, and otherwise every table a statement read, as SQLite's authorizer reports them. See [reading one row](#reading-one-row-by-its-key).
- **Changeset**: the rows an op's transaction changed, with their old and new values, as a SQLite session records them. It is the op's undo record, and the rows it holds, each named by its table and primary key, are the op's **write set**.
- **Late op**: an op that sorts before ops this device has already applied.

## The shared order

Every replica applies ops sorted by time, then device, then op hash.

- **Time** comes from a hybrid logical clock, in milliseconds. A device's clock never repeats or goes backwards, and it takes in the time of every op the device receives, so every op it makes afterwards sorts after them. An op therefore always sorts after every op its author had seen.
- **Delivery** will come from p2panda, which delivers ops in causal order: an op arrives only after the ops it depends on. Causal order is partial: two ops made concurrently, neither having seen the other, are left unordered. The clock flattens it into one total order consistent with it, and breaks the tie between concurrent ops by time, then device, then hash.
- **Received times are admitted first.** A time at or above 2^53 ms is refused for good, the same on every replica. A time more than five minutes ahead of this device's wall clock is refused only for now, since another device's wall clock may already have reached it: its sender offers it again later. Only an admitted time enters the clock, so one peer's fast clock cannot drag every replica's times forward.
- **Why not order by p2panda's causal graph instead**: it would still need a tie-break for concurrent ops, the clock already respects it, handlers need a real timestamp, and the order must work without p2panda, in single-node mode and in tests.

## Applying an op

The next op to apply is the first pending op in the shared order. It runs in a transaction of its own; when its handler settles, the core records its result, read set and changeset and commits, or, when the handler refused, rolls back and records the refusal with its read set.

An op is late when there are results for ops after it. The clock decides that, never the read set. Ops arrive late when a device that was offline comes back, or when two devices write concurrently.

## A late op is tried in place first

The late op runs on today's state, in its own transaction. Before it commits, the core compares its read set and write set with those of every op after it. Writes are known by row, from the changeset; reads by row when a statement looked one row up by its key, and otherwise by table, which reads every row of it. It keeps the run when:

1. no later op wrote a row the late op read, so it saw exactly the state at its own place;
2. no later op read a row the late op wrote, so nothing after it would have behaved differently; and
3. no later op wrote a row the late op wrote, so neither overwrote the other.

When all three hold, applying it in place gives the same state as applying it in order, so it commits and nothing re-runs. Ops that only read the same tables, ops that insert different rows into one table, such as two people posting messages, and an insert before an edit that looked up another row by its key apply in place.

## Reading one row by its key

SQLite's authorizer reports which tables a statement reads, never which rows, so the core works out whether a statement reads just one row. Finding that row may only narrow a read, never widen what is trusted: whatever the core cannot confirm counts as reading the whole table, so a mistake costs a rewind, never a wrong state. A statement reads one row only when:

- it is one SELECT, UPDATE or DELETE of one table, with no join, subquery or CTE, whose WHERE clause holds `column = value` terms joined by AND that name every primary key column, each value a literal or a parameter;
- SQLite's query planner plans it as one search of that table by its whole primary key, which rules out a view, an OR, a range and a key compared under another collation;
- the key compares its values exactly (SQLite's BINARY collation), so the value looked up names the one row found;
- the table has no triggers, whose reads the authorizer reports as the table's own;
- at each run, every key value bound already has the type its key column stores, text for a TEXT key and an integer for an INTEGER key, so it names the row as the changeset does;
- and the authorizer saw that run read only that table.

Each statement is worked out once per text and kept in memory per service, until SQLite's schema version changes; after a restart each is worked out again. Only the core's first look at a statement asks the planner, and every run is still compiled once, so the authorizer checks it.

Everything CarbonDB reads in a service's SQL lives in `crates/core/src/sql/`. A service's statements run as written; sqlparser is used in one place, `lookup.rs`, which parses a statement, and reads its tokens to number its parameters, to find the row it looks up. A statement sqlparser cannot read, such as SQLite's `x IN table`, runs all the same and reads its whole tables. A better way to read SQL would replace that one file.

## Otherwise the service rewinds

When the check fails, the late op's run rolls back and the service rewinds:

1. **Choose what to undo.** Every later op that read or wrote any of the service's tables. An op that touched nothing neither depended on the late op nor changed what it sees, so it stays. A rewind never reaches another service: what it did is its own, and a message it was sent stays sent.
2. **Undo them.** In one transaction, the chosen ops' changesets are inverted and applied, newest first, and their results are forgotten. They are pending again. Effects the late op's in-place run made are kept on its row, so its run at its place replays them.
3. **Apply in order.** The late op is now the first pending op, so it applies at its place, then the undone ops re-run after it, in order. Each is checked in place like any late op, against the ops still applied after it.

An undone op that re-runs may now succeed where it was refused, or be refused where it applied: its result is replaced. Whoever made the call was answered when it first applied; a later change of outcome is reported through [rejected after merge](../stories/backlog/rejected-after-merge.md), not built yet.

## Effects are made once

Every replica re-runs every op's handler, but what the handler does outside its own tables happens once: on the op's first run, on the device that made it.

- **Calls between services.** A call to the same service runs inside the op, under a savepoint. A call to another service the channel names is a message: the `call` effect posts it to that service, as an op there, and the answer is recorded. The answer is what the other service answered on this device at that moment; a late op there can still change its outcome, and if the caller then fails, what the other service did stays. Only delivery is guaranteed between services.
- **Recorded on the row.** Each effect's output, success or failure, is kept in order on the op's log row when its first run settles, whatever its outcome, and travels with the op. The core never interprets an output; the service decides what is a failure.
- **Replayed after.** Every later run, on another device or after a rewind here, gets the recorded outputs in order instead of making the effects again, even when the handler now asks for something else. A run asking for more effects than the first run made gets an unreachable failure, the same on every replica.
- **Made again when declared.** A service may declare an effect `replicate = "always"` in its `service.toml`; it is made on every run on every replica and never recorded.
- **Unreachable when this device cannot make it.** Single-node, this device makes what it can alone: calling a service it holds and creating a new service's space. An effect that needs peers, such as adding a member or a handshake with another device, has no handler and fails as unreachable.

`[@carbon]` runs only on this device, in the system log, which never replicates; what it changes in channels and services it changes through their own calls, as effects.

## What stays on each device

The read set and changeset never travel with an op. Each replica works out both when it applies the op, keeps them with its result, and replaces them when a rewind re-runs it:

- **Redundant**: every replica computes them anyway.
- **Wrong after a rewind**: what an op reads and writes depends on the state at its place, which a rewind can change.
- **Untrusted**: a peer could send false ones.

What travels is the log row: the op, and the effects its first run made.

## Watches

A watch re-runs its GET after every commit that wrote a table it read. Undoing ops in a rewind is such a commit, and so is every op that re-runs after it, so a watch may briefly show the state between them; once the rewind settles, it shows the settled state.

## Rules this depends on

- Handlers take time, ids and randomness from the op, never from the wall clock or a random source, so every replica draws the same values.
- A row's key comes from the op that creates it, never from SQLite, since a key SQLite assigns depends on the rows already there and can move after a rewind. The core refuses a service whose tables take their keys from SQLite.
- A handler that needs a value to be unique checks with a query, never with a constraint. A query is a read the in-place check sees; a unique key, a foreign key or `INSERT OR IGNORE` decides from rows already there without any read SQLite reports, and a write it refuses leaves nothing in the changeset, so a late op it refuses could keep a run that differs from its place and replicas would diverge. Constraints may stay as a backstop that never fires.
- All mutations go through the op log; SQLite state is a fold of the ops in the shared order.

## Not built yet

- The `append` tag, with which a late insert-only op would apply in place even when a later op read its whole table; today an insert rewinds the later ops that read its whole table, such as a search by another column.
- A synced op that times out pausing its service and retrying, instead of being refused, since a timeout differs between replicas.
- Checking received ops at one boundary: their hash recomputed, and their device and author verified.
- Effects that need peers, which p2panda will carry out: members, invitations, joining and leaving, enrolling a device, sync.
- Effects mediated by the WASM boundary for JS services; today a JS handler reaches effects through `effect` and `call`, and nothing stops it doing more.
- Pruning old changesets, and a window bounding how far back a rewind reaches (see [old ops](../stories/backlog/old-ops.md)).
