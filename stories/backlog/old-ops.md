# Old ops are refused or re-run safely across versions and time

Status: backlog

## Idea

Two things make an old op dangerous to apply or re-run: the handler code has changed since it first ran, or its device has been offline so long that merging it would rewind too far.

### Handler versions

A rewind re-runs later ops with the handler code a replica has now. If that code changed since the ops first ran, a replica that rewinds gets different results from one that never did, and replicas drift.

CarbonDB's side of the contract stays simple:

- CarbonDB hashes each handler, and the op records the hash of the handler that applied it.
- If the handler for that hash no longer exists on a replica, CarbonDB refuses the rewind, or the offline op, for that version.
- The handler call receives the hash, so a service can choose to refuse an op from another version, or migrate its input to the current version.

Services may grow richer on their side, for example a package that ships several versions sharing a tree of call and model implementations. CarbonDB does not need to know.

### Cutoffs

- App releases form a natural cutoff for old offline ops: ops from a version the app no longer ships are refused.
- A time window is supported as well: a device offline beyond it starts over rather than merging. The window also bounds how far back a rewind can reach.
- The window cannot be measured by each receiver's clock, because clocks differ between replicas and they would disagree about which ops to refuse.

## To decide

- How every replica applies the same time cutoff, for example through checkpoint ops published by a manage device.
- When old ops can be pruned, such as old chat messages.
- Whether a service that migrates an old op's input can guarantee its effect is identical, byte for byte, to what the old handler would have written. If not, migration risks the same drift it is meant to prevent.
- What is hashed: the handler alone, or the handler with the modules it calls.
- How a refused op or rewind is reported, likely through the `errors` space (see [rejected after merge](rejected-after-merge.md)).
