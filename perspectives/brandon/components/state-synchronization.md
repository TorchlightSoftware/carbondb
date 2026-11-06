# State Synchronization

Critical path: yes. Depends on: none.

## Context

- Wiki State Synchronization lists CRDT options: Automerge, Yjs / y-crdt, MaidSafe crdt_tree, rust-crdt; with an open task to prototype state sync on yjs.
- p2panda: append-only logs, broadcast-only and offline-first; Rust crates on iroh 1.x, pre-1.0 with APIs explicitly not yet stable.
- Freenet: its core feature; every contract defines an idempotent, commutative merge with summary/delta sync.
- nostr: replaceable events and MLS epochs; no general CRDT layer.
- Holochain: append-only source chains with an eventually consistent DHT.
- Autonomi 2.0 dropped registers and scratchpads; only paid pointers are mutable; x0x offers CRDT key-value stores.

## Agreed

None yet.

## Open questions

- Is history per device (Signal model) or synced across devices (Matrix model)?
- Depend on p2panda pre-1.0 crates, or use iroh alone and treat p2panda as a reference design?
