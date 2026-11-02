# Replicas are proven to hold the same ops when a member is removed

Status: backlog

## Idea

p2panda discards a message whose author has lost write access by the time it is processed, so a message sent concurrently with its author's removal could survive on one replica and vanish on another. CarbonDB's rule is that a removal op carries a cutoff, and every replica discards the removed member's ops above it.

A test proves this holds on real p2panda nodes.

## Acceptance

- With three nodes, a message and its author's removal are delivered in both orders, and every replica ends with the same state.
- Ops at or below the cutoff reach every replica, even one that has already processed the p2panda removal.
