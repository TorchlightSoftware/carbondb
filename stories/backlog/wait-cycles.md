# A write that would wait for itself through other services is refused at once

Status: backlog

## Why

An op that writes into a service whose running op is waiting on it can never be answered: the write queues behind the op waiting for it, and both sit until the caller's deadline, a `504` that tells nobody what went wrong. The core refuses the cases it can see at once with `508 LOOP_DETECTED` (`waits_on` in `crates/core/src/calls/dispatch.rs`):

- **Back along the call chain.** A write into a service whose op made the call, directly or through other calls.
- **Into a direct reader.** A write into a service whose running op is waiting, through `callsync`, to read the writer's service, or a service whose op waits on the write's call.

## What still waits until the deadline

A cycle with a `callsync` wait anywhere but its last step:

- `[books]`'s op, placed at 10:00, reads `[ledger]` through `callsync`, and waits for `[ledger]`'s ops placed before it.
- `[ledger]`'s op at 9:58 reads `[identity]` through `callsync`, and waits for `[identity]`'s ops placed before it.
- `[identity]`'s op at 9:55 writes into `[books]`. The write queues behind `[books]`'s op, which waits on `[ledger]`, which waits on `[identity]`, which waits on the write.

The check looks only at whether `[books]` waits to read `[identity]` itself, so the write is posted, and all three wait until a deadline passes.

## To decide

- Whether to find every cycle: follow who waits on whom across stores (`callsync` waits, held calls, ops queued behind a running one) from the service written to, and refuse the write when the path reaches the writer.
- Whether a deadline is enough for these longer cycles, given how rarely three services read and write each other in a ring.
