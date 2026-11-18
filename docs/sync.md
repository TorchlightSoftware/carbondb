# Sync: how devices carry ops over p2panda

This is for anyone working on how CarbonDB moves a channel's ops between devices: which p2panda node does what, how a channel's p2panda space comes to exist, what this device publishes and when, what it does with what arrives, how a new device joins an identity, and how a device comes to hold the channels it should. How replicas then reach the same state, whatever order ops arrive in, is [replication.md](replication.md).

What is built is the flow a person first takes: found an identity on one device, enroll a second device in it, add a service to the personal channel, and use it from both. The tests in `crates/core/src/tests/milestone.rs` run that flow with two devices in one process, over mDNS alone, through a relay with mDNS off, and with a configured bootstrap. What comes next is planned in [stories/p2panda-sync.md](../stories/p2panda-sync.md).

## What p2panda gives, and what it does not

CarbonDB uses p2panda 0.8.1, on iroh 1.4.1:

- **Causal delivery, not a total order.** Each device's log arrives in the order it was written, and an operation is held until the operations it depends on have arrived. Concurrent operations stay unordered, so CarbonDB's shared order and rewind are still needed.
- **Space ids are p2panda's.** `Node::create_space()` picks the id itself.
- **Write access must be direct.** A device publishes into, or changes the members of, a space only as one of its direct members.
- **Acknowledgements.** With the explicit ack policy, operations not yet acknowledged are offered again when the stream reopens; an ack is a high-water mark per author log.
- **Own messages echo back**, marked as published here; publishing into a space answers no operation hash.
- **Credentials** are an Ed25519 signing key and an X25519 identity secret. They can be generated or deserialized, not built from keys given to p2panda.
- **No direct channel between two devices.** An ephemeral stream broadcasts signed, unstored messages to whoever subscribes to its topic, 4096 bytes by default.

## Nodes

Each identity on a device runs one p2panda node, signing with its device key, with a store file of its own named by that key (`<device key>.p2panda.db`), since a device joining an identity starts its node before it knows which: a node takes one set of credentials, and identities on one device must stay unlinkable. A device's key pair is generated as p2panda credentials and kept, in p2panda's serialized form, as the device's secret in the system database, which no space ever holds (`crate::keys`). A node runs for each identity the system database lists, started by a watch on `GET [@carbon]/identities`, so at start and as identities are founded; an identity being founded starts its node first, since that node creates the p2panda spaces of its channels. A single-node CarbonDB (`mode = "none"`) runs no node.

Everything else of an identity sits in a directory of its own in the data directory, named by its id: the log of each channel it holds, and the store of each service there (`crate::channels`). Which channels this device holds is which logs are there.

Every node in the process runs on one multi-threaded tokio runtime of its own (`crate::nodes::runtime`), never on the core's thread, which runs SQLite, handlers and rewinds and would stall the network's timers and sync sessions. Whatever a node does reaches the core as a task on the core's thread awaiting it, the way any other thread's work wakes the core; the tests' task driver handles it the same way.

## Network settings

CarbonDB fixes its own network id, the explicit ack policy and the bind addresses. The TOML file's `[sync]` section names the rest, each optional:

- `relays`: iroh relays every identity's node on this device uses. An iroh relay helps open connections through NAT and forwards encrypted traffic; it holds no data and has no p2panda key. A relay named here serves every identity on the device, so whoever runs it sees their nodes connect from the same address and can link them.
- `bootstrap`: nodes to begin discovery from, each a node id and the relay that reaches it. Until an identity records a choice of its own, its node uses these.
- `mdns`: `active` (the default), `passive` or `off`.

CarbonDB names no relay or bootstrap of its own. Neither is needed on one LAN, where a code plus mDNS is enough; across networks a relay is, since without one a node is reachable only at an address its peer already knows. An enrollment code stands in for bootstrap: it carries the new device's relay URL, which the device scanning it adds at once.

## Channels and their spaces

Each channel is one p2panda space, whose messages are the ops of the channel's one log, addressed to every service in it, `[channel]` and `[@carbon]` included: each as its log row holds it, with the effects its first run recorded and its cause. Each service keeps its own store, with its tables and a pointer to each op addressed to it (see [replication.md](replication.md)). A channel's members hold all its services.

A channel's id is its p2panda space's when this device syncs, and derives from the op that created it single-node; a service's store is named by an id derived from the op that created it. Only `[@carbon]` creates either, by its `create` effect, and then names what it created by the channel's own ops, `POST [channel]/services` or `POST [channel]/ref`, from which every replica reads the id. `[@carbon]`'s ops replay on every replica like any op, their effects' recorded outputs with them, so a replica creates nothing again: it holds what the channel's committed state names. When a `[@carbon]` op is refused on its first run, what its `create` effects made is deleted and its p2panda space no longer followed.

The system is a local channel, its log never published: it has no p2panda space.

## Publishing

A channel's log is its outbox. After every commit in a synced channel, and when it is first followed, this device publishes its own ops that are publishable and not yet published, in the shared order (`crate::nodes::publish`): those whose outcome here is applied, and whose cause applied too, all the way up, so a failed cause's calls never leave the device. Once p2panda accepts one, the core records it in the log (`carbon_published`). A rewind can make an op publishable late and out of log order, which is why each op is recorded by hash rather than by a position in the log, and a rewind, which forgets outcomes, leaves that record alone. A crash between publishing and recording publishes the op once more, and peers drop the duplicate by op hash.

## Receiving

Every message that arrives on a channel's space passes one check before its log, which decides whether it is an op at all (`crate::nodes::receive`); once it is, it takes the same path as every op, through `Carbon::receive`, and is acknowledged once its row is logged. One addressed to a service this channel does not name here yet waits in the log until it does, and the service's store takes it up when it opens. The check:

- recomputes the op's hash from its envelope and time;
- checks that its device is its p2panda author;
- checks that its device belongs to its author identity: the identity's first device, whose key the identity's id derives from, or a device the identity's `[identity]` lists on this device.

Nothing more: the p2panda signature covers the effects, which the op's hash does not, and the version is checked when the op applies. What does not pass:

- A message that fails is kept as evidence in its channel's log (`carbon_error`), never applied, and acknowledged once kept: keyed by its p2panda operation hash, since the op hash it claims is unproven and logging under it could block the genuine op, with the signed operation, its author, when it arrived and which check it failed.
- A message whose device is not yet listed is held, unacknowledged, and offered again after every commit, since the op listing a newly enrolled device can arrive later, from another space; after two hours it fails as an unlisted device.
- A message more than five minutes ahead of this device's clock is held and offered again once the clock has caught up.

An author's later messages wait behind one of theirs that is held, since an acknowledgement is a high-water mark per author log. A device's own published op that p2panda echoes back is already logged, and is acknowledged at once.

## Loading channels

In the `ref` sync mode, the default, a node holds each identity's own channel, and every channel the channels it holds point to, with every service they name (`crate::nodes::load`). It watches `GET [channel]/refs` and `GET [channel]/services` on each channel it holds; a watch answers as soon as it is registered, so the same watches load everything at start and whatever appears later, as the ops naming it apply. A channel this device does not hold yet is created here and its p2panda space joined, which completes once a device of the identity adds this one; a service's store is opened, running the package registered under its name, and takes up what the channel's log addresses to it. The `all` and `manual` modes are not built.

## Enrolling a device

The new device shows a code, and a device already in the identity scans it (`crate::nodes::pairing`; [story 2](../stories/02-add-device.md)):

1. `POST [@carbon]/devices`, on the new device's system handle, makes its key pair, starts its node and answers `{code, device}`. The code carries the device's p2panda member, p2panda's 305-byte encoding of its device key and key bundle, cross-signed; a pairing secret; and its relay URL if it names one.
2. `POST [@carbon]/enroll {code, name, tags}`, on the existing device's handle of the identity, is a `[@carbon]` op in the identity's own channel. It lists the device in `[identity]` with `POST [identity]/devices`, signed by its `sign` effect with this device's key, which `[identity]` checks against the op's device; then its `enroll` effect has the identity's node register the new device's key bundle and add it to every channel's space the node follows, and the call answers. The node then answers the new device's announcements on an ephemeral topic the pairing secret names with the identity and its channels, sealed with that secret, until the code expires. The op replays on the identity's other devices, its effects recorded.
3. The new device announces itself on that topic until the identity arrives, then its core records the identity by an op on its system handle (`POST [@carbon]/identity` with `joining`), its node becomes the identity's, and the `ref` sync mode loads the rest. The app sees it in `GET [@carbon]/identities`; the core can watch that, but the app's system handle offers no watches yet. A code expires after ten minutes.

## Which devices hold which channels

For now every device an identity's `[identity]` lists is a member, at `manage`, of every channel's space the identity's node syncs: its own and personal channels, and those added after a device enrolled (`crate::nodes::members`). The device whose `[@carbon]` op calls for a change makes it, once: enrolling a device adds it to every channel's space the identity's node follows, and a channel `[@carbon]` creates gets every listed device as it is followed. Whenever a device follows a channel's space it also adds the listed devices missing from it, which repairs what a crash cut short; adding a member already added changes nothing. The assignment policy that will place an identity's devices by their tags is planned in [stories/p2panda-sync.md](../stories/p2panda-sync.md).
