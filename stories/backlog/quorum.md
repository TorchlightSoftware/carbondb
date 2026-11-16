# A caller can wait for more than one device to hold its op

Status: backlog

## Why

A call answers once this device has applied its op. The answer is this replica's at that moment, and a late op from another device can still change it. Some calls, such as a payment or an order, may want more assurance before the app tells the person it is done.

## What we know

- p2panda acknowledgements are a node's own high-water mark per author log, for replay when a stream reopens; p2panda exposes no count of the peers that hold or applied an op, and each replica's result is its own ([p2panda sync](../p2panda-sync.md)).
- p2panda's ephemeral stream broadcasts signed, unstored messages to a topic's subscribers, 4096 bytes by default, which replicas could use to send their result for an op.
- A quorum answer is still provisional: a replica that was offline can later deliver a late op that changes it.

## To decide

- Whether to offer a quorum setting on the service-call binding at all, and whether it is worth building before an app needs it.
- What it waits for: N replicas holding the op, or N replicas reporting the same result.
