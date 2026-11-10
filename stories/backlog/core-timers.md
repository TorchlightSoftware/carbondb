# The core keeps its own timers once replication runs

Status: backlog

## Idea

Every call must settle within its service's timeout. The core decides each deadline and what expiry means, but in single-node mode it has no event loop of its own: it runs on the host's JS thread, so the host sets the alarm, on Node with `setTimeout`, and cancels it when the call settles.

Replication runs p2panda on a tokio runtime, and ops from peers reach the core through a crossing that wakes its thread. Once that crossing exists, deadlines can be tokio timers that wake the core through it, and the host no longer schedules them.

## To decide

- Whether the host then provides only the crossing onto the core's thread.

## Related

- Timeouts and refusals in [implementation.md](../implementation.md).
