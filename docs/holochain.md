# CarbonDB and Holochain: one shared order, or none

This is for developers who know Holochain, or are choosing between the two, and want to know where CarbonDB differs, what each can promise, and what each costs.

Both let a group of people share data without a server, each device keeping its own copy and accepting changes made offline. They differ on one question: does a change depend on the order changes happened in? Holochain says no, and builds everything so that order never matters. CarbonDB says yes: it puts every op of a channel in one shared order, applies each against the state at its place, and redoes work when an op arrives late. Most other differences follow from that.

## What each can promise

Holochain checks an action only against what it names by hash and against its author's own chain. That check gives the same answer on any node at any time, so a valid action stays valid forever, nothing is undone, and late data is always accepted. Missing dependencies leave an action waiting, not refused.

CarbonDB runs a service's handler against the state at the op's place in the shared order ([replication.md](replication.md#the-shared-order)). A rule can read anything in the service's tables, as a server's handler would. When an op arrives that sorts before ops already applied, the service rewinds what it conflicts with and runs those ops again in order ([replication.md](replication.md#otherwise-the-service-rewinds)).

| Rule | Holochain | CarbonDB |
|---|---|---|
| A post is by its author | yes | yes |
| Only a bookkeeper may post | as the author saw it | as of the post's place |
| A name is claimed once | no | yes |
| A balance never goes negative | no | yes |

Rules over current shared state, such as a name claimed once, cannot be kept without an order or coordination: this is known as invariant confluence (Bailis et al., *Coordination Avoidance in Database Systems*, 2014). Holochain gives them up to keep every check timeless. CarbonDB keeps them, and pays with rewinds.

A Holochain post made while its author's bookkeeper role was being revoked elsewhere stays valid, since the author had not seen the revocation. In CarbonDB the post is refused if the revocation sorts before it, even when the revocation arrives later ([replication.md](replication.md#consistent-reads)).

## Programming model

CarbonDB services keep SQL tables and handle calls like a server: read rows, check rules, write rows. A read of another service through `callsync` answers as of the reader's place, much as Holochain's `must_get_*` reads by hash: what a read sees is fixed by the op, not by when it ran.

Holochain apps work in entries, links and actions, split into an integrity zome (pure validation, whose hash defines the network) and coordinator zomes (code with effects). Current state is not stored; an app builds it at read time from entries, links, updates and deletes that never change, and resolves conflicting updates itself.

The surprise in CarbonDB is that a result can change after the fact: an op applied today can be refused tomorrow when an earlier op arrives, or the reverse. The surprise in Holochain is that the data never changes, so the app has to decide what conflicting data means.

## What it costs

| Situation | Holochain | CarbonDB |
|---|---|---|
| Everyone online, writing | a local write, then receipts from other nodes | a local SQLite transaction |
| Reading current state | built from links at read time | a table read |
| A device back after a day | each action validated once | one rewind from its earliest op |
| A late op on busy rows | validated once | re-runs every later op that used those rows |

- **Steady state favors CarbonDB.** An op is one SQLite transaction on each device. A Holochain action is published to the nodes responsible for it, which validate it and send receipts; the author waits for five per DHT operation by default.
- **Late ops favor Holochain.** Its cost per action never depends on arrival order. In CarbonDB, late ops logged together cost one rewind from the earliest of them, and only ops that read or wrote a row the late op changed are undone; a late op that touches nothing later ops used applies in place ([replication.md](replication.md#a-late-op-is-tried-in-place-first)). The worst case is an old op on rows every later op read.
- **Reads favor CarbonDB as history grows.** A Holochain query over a busy link base returns more with every link added; a CarbonDB table holds only current rows.

## What each device keeps

Holochain shards data across a DHT: a node keeps its own chain, the DHT operations whose addresses its arc covers, and a cache. In a group small enough that every node covers everything, every node keeps every action, once.

A channel is CarbonDB's unit: every member's device keeps all of it, suited to groups, not to the open network. It keeps the data three ways: the channel's op log, the service's tables, and the history of each table in its store's WAL, which consistent reads need ([replication.md](replication.md#what-stays-on-each-device)).

## Time and trust

Both take an action's time from its author. Holochain checks it against the author's previous action. CarbonDB refuses an op dated too far ahead of the receiver's clock, but checks nothing against the author's previous op, so a member can date an op back to move ahead of another; the shared order follows that date.

## How long a device may stay away

Holochain needs no limit: a late valid action is accepted whenever it arrives. CarbonDB needs one. Without it, an op from long ago makes every device rewind back to it, and every device must keep the history needed to read at its place. What that limit is, and what happens to ops beyond it, is not decided yet.
