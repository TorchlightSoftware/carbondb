# Effects made outside an op's transaction, by replay

Status: backlog — a spike, to measure before deciding

## Problem

An op runs in one SQLite write transaction, from its first statement until its handler settles, awaits included (`BEGIN IMMEDIATE`, `crates/core/src/store/mod.rs`). SQLite allows one write transaction per file at a time, and each service is one file, so a write that waits on an effect, such as a `fetch` or a call to another service, holds its service for the whole wait: every other write to that service queues behind it. Reads are unaffected; each runs on its own snapshot.

## Idea

Make effects with no transaction open, using the replay every op already supports:

1. When an op's first run asks for an effect that is not recorded yet, the run rolls back and frees its service.
2. The effects it asked for are made with no transaction open, several at once when it asked for several before awaiting any, as `Promise.all` does.
3. The op runs again with those outputs, as any replay does. A replay never waits, so each run holds the service only for its own computation.
4. Meanwhile later ops apply. When the waiting op returns, its place is checked as a late op's is (`rewind::fits_in_place`), and the service rewinds only when they overlap.

This takes the waiting out of the serialized part; it does not change the shape of the serialization: an op's own computation and statements still run one at a time per service.

## Costs

- A handler runs once more for each round of new effects. Handlers must already give the same result when re-run, since every replica re-runs them, so this costs time, not correctness.
- An effect declared `replicate = "always"` is made on each of those runs.
- Ops that overlap rewind more often under contention.

## The spike

Measure, before deciding:

- Write throughput per service with effects of realistic latency (tens to hundreds of milliseconds), today against this design.
- The cost of the extra runs for handlers with one, several, and sequential effects.
- How often overlapping ops rewind in a contended workload, such as many writes to one chat channel.
