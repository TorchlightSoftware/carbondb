# Reads are consistent within a channel

Status: implemented — pending review: `callsync` in the core, its as-of reads rebuilt from changesets, waits and reader re-runs, tested as cases 1, 3, 6, 8 and 12 and the example under "Reading the past" ([replication.md](../../docs/replication.md#consistent-reads), [wal-vs-changesets.md](../../docs/wal-vs-changesets.md)), and in the JavaScript package; not built: `call`'s own timeout, the cost cases 5, 7 and 9–11, and pruning changesets and the op log

## Why

Within one service, an op reads and writes in one transaction at its place in the shared order, and a late op that changes what it read re-runs it ([replication.md](../../docs/replication.md#a-late-op-is-tried-in-place-first)). Across services in a channel, writes are consistent: a call follows the op that made it, its cause ([replication.md](../../docs/replication.md#causes)). Reads are not. Today a GET one service makes of another inside a write is answered from the other service's current snapshot on the author's device, recorded as a `call` effect, and replayed from that record on every other replica, so every replica agrees on an answer that may be stale or forged. Access control is the first need: `[books]` must check `[identity]`, an app's role service in the channel as Otto's identity service is, and have that check hold on every replica.

## The design

A service chooses, call by call, between two ways of calling another service in its channel.

### `callsync`: consistent with the caller's place

- **Reads at the caller's place.** A GET made through `callsync` inside a write is answered at the caller's place in the channel's shared order, on every replica, and what it read is kept in the service read as read by the caller's op, which an op applied there at an earlier place runs again. Its own effects are recorded on the caller's row and replayed, so it need not be pure.
- **Only the reader waits.** The op waits until the service it reads has applied every op before its place. A service that depends on another bears its slowness and failures: if `[identity]` is behind, `[books]` waits; if `[identity]` is stopped, `[books]`'s readers stall. A service that reads no other, such as `[chat]`, applies side by side and never waits.
- **Waiting counts against the deadline.** Time a `callsync` spends waiting on another service counts against the caller's timeout.
- **The service read is never held back.** When `[identity]` is already ahead of the reader's place, each row is read as of that place, rebuilt from the changesets of `[identity]`'s later ops, without pausing `[identity]` ([Reading the past](#reading-the-past)).
- **Reads by id only.** A statement that reads a whole table of another service is refused, because it cannot be read as of a place row by row. Whether a statement reads one row is found at run time, as for every read: the SQLite authorizer reports its tables, and [lookup.rs](../../crates/core/src/sql/lookup.rs) narrows it to one row when the query planner confirms a search by whole primary key. The verdict is the same on every replica running the same version.
- **An earlier op re-runs only its readers.** Any op `[identity]` applies at a place before a reader that already read a row it writes re-runs that reader, and nothing else, whether or not the op is late within `[identity]`; `[identity]` itself rewinds as it does today.
- **No deadlock.** Every wait targets an earlier place in the shared order, so two services reading each other never wait on each other forever. A write into the reader's service, made by an op the reader waits on, would queue behind the reader; it is refused at once (`508 LOOP_DETECTED`).
- **Writes keep their cause.** A write made through `callsync` is bound to the op that made it and follows it, as calls within a channel do today.

### `call`: an effect, made once

- **Made once and recorded.** A call made through `call`, read or write, is an effect: made on the op's first run, its answer recorded on the row, and replayed on every later run. It never joins the caller's read set.
- **No cause.** A write made through `call` is a message bound to nothing. When the caller is later refused, the write stays; undoing it takes a compensating call, as in a saga. The write is an ordinary op in its own service, applied in that service's order like any other.
- **Searching, then acting by id.** A service that needs to search another searches through `call`, then reads or writes the ids it chose through `callsync`. An app can do the same from outside any op.
- **Its own timeout.** A `call` does not count against the caller's timeout; it has its own default timeout.

## Reading the past

### Two logs, and which one reads the past

- **The channel's oplog** holds ops in the shared order, one log per channel, shared with peers: what was meant to happen.
- **Each store's changesets** in `carbon_result` hold row-level undo records, kept only on this device, which rewinds invert, and which a `callsync` GET undoes on one row to read it as it stood.

A `callsync` GET's handler runs its own SQL unchanged, each statement against the one row it looks up, rebuilt and held alone on a scratch connection. SQLite checkpoints each store's WAL as usual. A kept WAL, read through SQLite snapshots, was built first and measured; it keeps about 500 times the bytes the changesets do ([wal-vs-changesets.md](../../docs/wal-vs-changesets.md)).

### One horizon

The time window of [old ops](old-ops.md) bounds everything: past it no rewind reaches before it, no reader needs a state from before it, and neither the oplog nor the changesets keep anything older.

### Each row's history is in order, though the whole database's is not

A store commits ops in the order this device applied them, not in their places in the shared order. A late op that touches nothing a later op read or wrote is applied in place, on top of current state, with no rewind ([replication.md](../../docs/replication.md#a-late-op-is-tried-in-place-first)). The database then holds a mix that never existed at any one place.

Three people are enough to show it, and fewer are not: the reader has to read two different rows, each changed by a different op on the other side of it.

- **Before:** Ann holds the inviter role in `[identity]`, and Dave manages it.
- **10:03:** Dave adds Bob to `[identity]` on his laptop, which is offline.
- **10:05:** Ann invites Bob to a chat, by the identity key Bob gave her. `POST [chat]/invitations` reads Ann's role row and Bob's member row in `[identity]`. Her device has not seen Bob added, so the invitation is refused.
- **10:07:** Dave revokes Ann's inviter role from his phone. Ann's device applies it; it sorts after the invitation, so the invitation does not run again.
- **Later:** Dave's laptop comes online and Bob's addition arrives late. It changes only Bob's row and the revocation changed only Ann's, so it applies in place, with no rewind. Ann's invitation read Bob's row, so it runs again as of 10:05, when Ann held the role and Bob was a member, and it goes through.

No commit on Ann's device holds both facts: every commit with Bob as a member has Ann's role revoked, and every commit with Ann's role lacks Bob. Read from any single commit, the invitation would be refused on Ann's device while replicas that applied the ops in order accept it.

One row's history is always in order. An op that writes a row a later op already read or wrote never fits in place; it rewinds ([rewind.rs](../../crates/core/src/ops/rewind.rs)). So each statement of a `callsync` GET, which reads one row by id, reads that row as it is now with the changes of the ops placed after the caller undone, newest first. Each row is right as of the caller's place, though no commit held them all. Finding those ops takes an index from each row to the ops that wrote it, by place.

Rejected: shadowing tables with rebuilt rows in temp tables on the reader, and a `WITH` clause named for the table, which loses column affinity and collation ([wal-vs-changesets.md](../../docs/wal-vs-changesets.md#running-the-statement-against-the-rebuilt-row)).

## Cases for consistent reads

Without `callsync`, replicas never disagree with each other: every replica replays the author's recorded answer. What goes wrong is that the answer they agree on can be stale or false, and nothing ever re-checks it.

### 1. A late role change leaves a write it should have refused

- **Setup:** Dave manages `[identity]`; Sarah holds the bookkeeper role, which `POST [books]/entries` checks by reading her role row in `[identity]`.
- **Steps:** Dave removes Sarah's role on his laptop while it is offline. Sarah, who has not seen it, posts an entry a minute later. Dave's laptop comes back online.
- **Through `call`:** Sarah's entry stays applied on every replica, because each replays the "allowed" her device recorded.
- **Through `callsync`:** every replica re-runs Sarah's entry after the role change, refuses it, and records the refusal.
- **Consequence:** a write by someone who no longer had the right to make it stands in the books.
- **How people find and fix it:** through `call`, only an audit finds it: someone compares the entry's author and time against the history of `[identity]` and posts a compensating entry by hand. Through `callsync`, the refusal appears among the rejected calls ([rejected after merge](rejected-after-merge.md)), and Sarah or a manager re-enters it if it should stand.

### 2. A member records a check that never passed

- **Setup:** as case 1; Martha is a member with no role.
- **Steps:** Martha's device, modified, writes `POST [books]/entries` with a recorded `[identity]` answer of "allowed".
- **Through `call`:** every replica replays the forged answer and applies the entry.
- **Through `callsync`:** every replica reads `[identity]` itself, finds no role, and refuses it; nothing recorded is consulted.
- **Consequence:** any rule enforced through another service's state can be bypassed by one modified device.
- **How people find and fix it:** through `call`, nothing distinguishes the forgery from an honest entry; an audit of every entry against role history is the only defence. Through `callsync`, there is nothing to find.

### 3. An invitation names a member who was removed concurrently

- **Setup:** `POST [chat]/invitations` takes member ids the app found with `GET [identity]/members` outside any op, and reads each id in `[identity]` through `callsync`.
- **Steps:** Ann invites Bob by id while, concurrently, Dave removes Bob from `[identity]`.
- **Through `call`:** the invitation stands, pointing at a member who no longer exists.
- **Through `callsync`:** the invitation re-runs after the removal and is refused.
- **Consequence:** dangling references between services, shown in the interface as invitations to nobody.
- **How people find and fix it:** through `call`, someone notices the ghost entry and deletes it; through `callsync`, the refusal is reported to Ann.

### 4. A payment is made against hours that were reduced

- **Setup:** `POST [books]/payments` reads Sally's approved hours from `[worklog]` by id and pays for them.
- **Steps:** a manager reduces Sally's approved hours on an offline device; the bookkeeper pays Sally against the old hours before the reduction arrives.
- **Through `call`:** the payment stands at the old amount.
- **Through `callsync`:** the payment re-runs at its place, sees the reduced hours and pays the lower amount, or refuses if the handler requires an exact match.
- **Consequence:** an overpayment, invisible until reconciliation.
- **How people find and fix it:** through `call`, a reconciliation report across `[worklog]` and `[books]` finds the gap, and a person posts a correcting entry; through `callsync`, the payment's changed result is reported to the bookkeeper. If the payment also sent money through `fetch`, that effect stays made either way, and the correction is a compensating call.

## Cases for what it costs

### 5. A slow dependency delays only its dependents

- **Normal operation:** `[identity]` is slow to apply a burst of ops while `[books]` reads it through `callsync` and three people chat in `[chat]`.
- **Expect:** `[books]` ops wait until `[identity]` has applied every op before their places; `[chat]` messages apply without waiting. Measure the delay each sees.
- **And when `[identity]` stops:** `[books]` ops that read it stall, and `[chat]` carries on.

### 6. A reader behind its dependency reads as of its place

- **Normal operation:** `[identity]` has applied ops past a `[books]` op's place, one of which changed Sarah's role.
- **Expect:** the `[books]` op reads Sarah's role row as it stood at its place, from the old value in that later op's changeset, without `[identity]` pausing; a row no later op wrote is read as it is now.

### 7. A role change while people are writing re-runs only the ops that read it

- **Normal operation:** Dave changes Sarah's role while three members post in `[books]`; his op sorts a few milliseconds before posts other replicas have already applied.
- **Measure:** how many `[books]` ops re-run on each replica. Each post read its author's role row by id, so only posts that read Sarah's row re-run: expected zero or one.

### 8. A whole-table read of another service is refused

- **Normal operation:** `POST [chat]/invitations` searches `[identity]` members by name through `callsync`.
- **Expect:** the search is refused, the same on every replica. The same search through `call` is answered and recorded, and the ids it found are then read through `callsync`.

### 9. A device returns after a week with a role change at its start

- **Extreme:** Sarah's laptop was offline for a week; its first op that week removed Ann's role, and Ann has written hundreds of entries across `[books]` and `[worklog]` since.
- **Measure:** ops re-run in each service, the time each spends rewinding, how long each accepts no new ops, and how many intermediate states watches show.
- **Mitigation:** the time window that bounds how far back a rewind reaches, past which a device starts over instead of merging ([old ops](old-ops.md)); rewinding in batches that keep the interface responsive; reporting what changed as one summary rather than one refusal per op.

### 10. A rewind cascades through causes and reads

- **Extreme:** a late `[identity]` op re-runs a `[books]` payment; the payment's outcome flips, so its `callsync` write into `[worklog]` is taken up again; ops in `[books]` that read that `[worklog]` row re-run in turn.
- **Measure:** the depth of the cascade and the ops re-run at each level.
- **Mitigation:** only a changed outcome or a changed row propagates, as causes already do; the in-place check keeps every op whose reads were untouched.

### 11. Every op reads one hot row

- **Extreme:** every handler in the channel reads `[identity]`'s settings row; a late write to it re-runs every op after it that read it, in every service.
- **Mitigation:** versioned rows: a setting change writes a new row under a new key, and ops name the version they act under, so a new version re-runs nothing that named an older one.

### 12. A write through `call` stays when its caller is refused

- **Normal operation:** `POST [books]/close` posts a notice through `call` to `[chat]`, then a late op refuses the close at its place.
- **Expect:** the close is refused on every replica, and the notice stays in `[chat]`; the same notice through `callsync` is undone with the close.

## Set aside

- Extending consistent reads into sub-channels, whose members may not hold the parent: consistent reads stop at the channel boundary for now.
- Skipping rewinds for changes close in time to the ops that read them, below a threshold.
