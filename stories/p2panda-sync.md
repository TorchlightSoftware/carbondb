# Syncing spaces over p2panda

Status: set aside — waits for rewind on a single device and the WASM host; the decisions below are agreed, the open questions are not

This plan carries a space's ops between devices and identities with p2panda 0.8, behind CarbonDB's identity-level API. It builds on two things that come first: rewind and replay on a single device, which keep every replica in one shared order, and a WASM host running every service, which keeps replicated handlers from repeating outside calls on every replica. [implementation.md](implementation.md) holds the wider design; where the two disagree about p2panda, this file wins.

## What p2panda 0.8 gives us, and what it does not

Checked against p2panda 0.8.1 and the spike's earlier pin:

- **Causal delivery, not a total order.** Each device's log arrives in the order it was written, and an operation is held until the operations it depends on have arrived, membership changes included. Concurrent operations stay unordered, so CarbonDB's shared order and rewind are still needed.
- **Space ids are p2panda's.** `Node::create_space()` takes no id and picks one itself.
- **Write access must be direct.** The high-level `Space` API checks the calling device against the space's direct members before it publishes, adds, removes, promotes or demotes, so a device whose access comes only through a group cannot write. Receivers do accept group-held access.
- **Acknowledgements.** With an explicit ack policy, operations not yet acknowledged are replayed when the stream reopens; an ack is a high-water mark per author log.
- **Own messages echo back.** A published message returns on the space's stream, marked as coming from this node; `publish` returns no operation hash.
- **Credentials.** A node's credentials hold its Ed25519 signing key and a separate X25519 identity secret, both of which must stay the same across restarts.
- **Removal.** A message from a member removed concurrently with sending it is dropped silently; earlier messages stay valid.
- **No direct peer-to-peer channel.** An ephemeral stream broadcasts signed, unstored messages to whoever subscribes to a topic, 4096 bytes by default.
- **SQLite.** p2panda's store uses sqlx 0.9.0, so rusqlite 0.39.0 keeps linking beside it.

## Agreed

### Dependency

- CarbonDB depends on p2panda 0.8.1 from crates.io instead of the spike's git pin.

### Spaces and ops

- In replicated mode a space's id is the id p2panda assigns when it is created; CarbonDB does not choose it. Single-node mode assigns ids itself. See [Creating a space](implementation.md#creating-a-space).
- An op is a message in its space's p2panda space. A received message is written to the op log by the space's OpWriter, like any other op, and applies in the shared order.

### Identities and devices

- CarbonDB's API operates in terms of identities: an identity is a member of a space, never a device. The identity model stands; only its wiring in p2panda changes.
- Under the hood, devices are added to each space directly in p2panda.
- An identity tags its devices, and a service tags its operational requirements.
- An identity's assignment policy reads the tags on its devices and on the services in a space, and assigns which of its devices hold that space at pull, read, write or manage. Much as a scheduler places jobs on suitable machines by their labels, the policy places a space's work on suitable devices.
- The CarbonDB core provides a default assignment policy. Letting an identity replace it is [backlogged](backlog/assignment-policy-override.md) until testing shows the use cases.
- Asking p2panda to accept group-held write access is [backlogged](channels/backlog/upstream-group-write.md), so that we can share a working use case first.

## Open

### The assignment policy

- Where a service declares its tags: in its `service.toml`, or elsewhere.
- What the default policy assigns, such as the access a device without matching tags gets.
- When the policy runs (a device enrolled, a tag changed, a space joined or created) and which device carries out the p2panda changes, once.
- How a replica ties a received op's device to its author identity, now that space access is per device: whether the identity's p2panda group still exists for that, or the identity space's `[device]/origin` entries are read instead.
- Whether the existing `manage` device tag becomes one input to the policy, or stays separate.

### The crossing into the core

The core is single-threaded and runs on the host's thread, asking the host for timers through `Schedule`. A p2panda node needs a tokio runtime and runs its own threads.

- What the crossing looks like to the core: one host-supplied hook like `Schedule`, carrying received ops and network events in and publish requests out. On Node, a napi ThreadsafeFunction can wake the core's thread; Rust tests can run the core and p2panda together on one thread.
- Whether call deadlines move onto tokio timers in the same change, settling [the core-timers backlog story](backlog/core-timers.md).

### Checking received ops

- What one boundary checks for every op received from a peer: its hash recomputed from its envelope, its device equal to the p2panda author, that device tied to the op's author identity, and anything else. Today `oplog::to_op` fills missing fields with defaults, which is safe only for rows this device wrote.
- How an op that fails the check is recorded, given that every failure is logged and reported the same way.
- How an op refused for now (`Carbon::receive` answers 425 when its time is more than five minutes ahead of this device's clock) is offered again: left unacknowledged, p2panda replays it only when the stream reopens.

### Nodes and credentials

One p2panda node runs per identity on a device, with its own store file, because a node takes one set of credentials and identities on one device must stay unlinkable.

- How the X25519 identity secret is generated and kept in the system database beside the device key.
- Which network settings (network id, relays, bootstrap, mDNS) the TOML file names.

### Publishing

The op log is the outbox: a publisher drains this device's applied ops, also after a crash.

- Where this device records that an op was published.
- Whether acknowledging a received op once it is written to the op log, rather than after it applies, is enough, since the log is the durable queue.

### Membership and enrollment

- [Story 7](07-add-members.md), inviting and admitting a member: what a contact holds now that an identity is not one p2panda group id, its devices or a way to learn them. Admitting an identity means admitting the devices its assignment policy places in the space, and p2panda needs each device's key bundle before it can be added.
- [Story 2](02-add-device.md), enrolling a device over an ephemeral pairing topic derived from a secret in the code, with the payload encrypted by us: how the device that showed its code learns the exchange finished, and which spaces the new device is added to, by which device, under the assignment policy.

### Status

- Where `GET [@carbon]/status` reads access from, now that p2panda access is per device and CarbonDB speaks in identities. It also reports sync as off, waiting or on, and the peers this device has synced the space with; the `ref` sync mode starts syncing a space the first time a ref reaches it.

## Build order

1. Move to p2panda 0.8.1, and run one node per identity with persisted credentials.
2. The crossing into the core, publishing applied ops, receiving ops into the op log, and acknowledgements; proved with two nodes in one process.
3. The assignment policy and its default, then inviting and admitting members (story 7).
4. Device enrollment (story 2).
5. Status, peers and sync modes.

## Docs this changes

Once the assignment policy is settled, these passages, which describe an identity as a p2panda group of its devices, are rewritten for devices added directly: the Identity term in [README.md](README.md); Identity and devices, No secrets in replicated ops, Publishing an identity and Invitations in [implementation.md](implementation.md); `POST [@carbon]/devices` in [services.md](services.md); [story 2](02-add-device.md) and [story 7](07-add-members.md); and the [revoke device](backlog/revoke-device.md), [paper key](backlog/paper-key.md), [security review](backlog/security-review.md) and [public identity proofs](backlog/public-identity-proofs.md) backlog stories.
