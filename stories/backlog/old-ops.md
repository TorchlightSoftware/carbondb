# Old ops are refused or re-run safely across versions and time

Status: backlog

## Idea

Two things make an old op dangerous to apply or re-run: the handler code has changed since it first ran, or its device has been offline so long that merging it would rewind too far.

### Handler versions

A rewind re-runs later ops with the handler code a replica has now. If that code changed since the ops first ran, a replica that rewinds gets different results from one that never did, and replicas drift.

CarbonDB's side of the contract stays simple:

- A device runs one version of each service at a time. It never keeps an old version around to apply old ops, or to re-run them in a rewind.
- Every op records the version it was written under: the package's hash, and the version the package declares.
- The handler receives that version and decides: it supports the op, migrating its input to the current shape when needed, or refuses it. The same holds for an offline op arriving late and for ops a rewind re-runs across an upgrade.
- A package declares its version in its `service.toml`, semver canonically. CarbonDB records it but does not trust it: a breaking change labelled as a patch shows up as ops the handler refuses, not through its label.
- Service code and schema only move forward. CarbonDB never reverts a schema to replay old ops under old code, since any gap in a service's migrations would turn into data loss.

Services may grow richer on their side, for example a package that ships several versions sharing a tree of call and model implementations. CarbonDB does not need to know.

For example, peers A, B and C run version 0.2.0 of a service, and C goes offline with ops it has not yet published. A upgrades to 0.2.1, whose schema renames a column, and writes an op; B upgrades and applies it. C comes back, upgrades, and applies A's op without trouble. A and B now receive C's ops, written under 0.2.0. Version 0.2.1 decides for each one whether to migrate its input to the new column or refuse it.

### Established practice

Handling old writes after a schema change is a solved problem in neighbouring fields, and services can follow the same patterns:

- **Upcasting.** Event-sourced systems keep old events exactly as they were and translate them into the current shape when they are read (Greg Young, *Versioning in an Event Sourced System*). A handler migrating an old op's input is the same move.
- **Lenses.** Ink & Switch's Cambria project translates data between schema versions with small two-way lenses, so local-first peers on different versions keep working together.
- **Expand, then contract** (also called parallel change). A breaking change is split in two: first add the new shape and keep the old one working, so writers on the previous version still succeed; remove the old shape only once no writer uses it. In a space, that is once no member still runs the old version.

One caution is particular to CarbonDB: an op is a call re-run through current code, not a recorded fact, so a migrated input has to lead to the same writes on every replica (see below).

### Cutoffs

- App releases form a natural cutoff for old offline ops: a version may refuse ops from versions it no longer supports.
- A time window is supported as well: a device offline beyond it starts over rather than merging. The window also bounds how far back a rewind can reach.
- The window cannot be measured by each receiver's clock, because clocks differ between replicas and they would disagree about which ops to refuse.

## To decide

- How every replica applies the same time cutoff, for example through checkpoint ops published by a manage device.
- When old ops can be pruned, such as old chat messages.
- Whether a service that migrates an old op's input can guarantee its effect is identical, byte for byte, to what the old handler would have written. If not, migration risks the same drift it is meant to prevent.
- What is hashed: the handler alone, or the handler with the modules it calls.
- How a refused op or rewind is reported, likely through the `errors` space (see [rejected after merge](rejected-after-merge.md)).
