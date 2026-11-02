# A person learns a call was rejected after merge

Status: backlog

## Idea

A call that applied on the author's device can be rejected once ops from other devices land ahead of it in the shared order. The op stays in the log as rejected, but the author is not told.

CarbonDB provides notifications and a queue of sync errors, kept in a dedicated `errors` space rather than the personal space. The person works through the queue with the op history interface, where a revert is a compensating op.

## To decide

- Whether the `errors` space replicates among the identity's devices, so every device sees the queue, or stays local to the device that found the rejection.
- What an entry holds: the rejected op, the space, the service error, and the op that caused it.
- How an entry is cleared, and how an app subscribes to notifications.
