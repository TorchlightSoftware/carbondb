# Syncing services over p2panda

Status: agreed — the first milestone is built (identity, enrollment, a service in the personal channel, chat between two devices; see [docs/sync.md](../docs/sync.md)); next, fetching packages from peers

This plan carries each service's ops between devices and identities with p2panda 0.8, behind CarbonDB's identity-level API. What is built is described in [docs/sync.md](../docs/sync.md): one node per identity, space ids from the `create` effect, publishing from the op log, the check on every received op, loading in the `ref` sync mode, and enrolling a device by the code it shows. [implementation.md](implementation.md) holds the wider design; where the two disagree about p2panda, this file wins.

## Agreed, not built yet

### Nodes and network

- A node's iroh relays also come from its identity's relay devices: a relay device is a pull-only device of the identity whose host also runs an iroh relay, so enrolling it gives the identity both at one address, and it serves only its own identity.
- An identity chooses its own bootstraps, from those the TOML file offers or others, and uses them to fill its address book and find public channels. A configured bootstrap is also a well-known place to post invites, identities and cryptographic claims; to hold what is posted, it runs CarbonDB and holds the spaces it is posted in. Where an identity records its choice is not yet defined; until then its node uses the bootstraps the TOML file offers.
- The mDNS configuration of the first milestone's test is skipped, with its reason, where multicast is unavailable, such as some CI containers. It runs everywhere for now.

### Receiving

- If `[identity]` later lists a device whose messages failed the check as an unlisted device, they are offered again from the signed operations `carbon_error` keeps. That needs a way to decode a kept operation's body again, which p2panda encrypts towards the space's members.

### Enrollment and the default policy

- The reverse direction: the existing device shows a code and the new device scans it, with `POST [@carbon]/identity {slug, code, device}`; which op records the new device's key, name and tags once that exchange finishes is not yet defined ([story 2](02-add-device.md)).
- The agreed design has the device that made an op carry out the membership changes it calls for, once. What is built has every device add the listed devices missing from the spaces it follows; adding a member already added changes nothing, so it differs only in who does it. The full assignment policy settles which.

### Sync modes

- `manual`: the app loads spaces itself, with `POST [@carbon]/sync/start`. `all`: not yet defined. Both are refused as not built.

## Open

### The assignment policy

- Where a service declares its tags: in its `service.toml`, or elsewhere.
- What the default policy assigns beyond the first milestone, such as the access a device without matching tags gets.
- When else the policy runs (a tag changed, a channel joined) and which device carries out those p2panda changes, once.
- How a peer in a shared channel ties a device to its identity without reading the identity's own channel: through the identity's p2panda group, or something else that peer can read.
- Whether the existing `manage` device tag becomes one input to the policy, or stays separate.

### Checking received ops

- How a channel policy turns the evidence in `carbon_error` into consequences, squelching or banning a device or identity past a threshold, and who carries them out. A failed hash or author check is the same on every replica, which receives the same signed operation, so such a policy can act on the same evidence everywhere.

### A sync API of CarbonDB's own

- Whether CarbonDB keeps a sync API of its own, independent of the transport, with p2panda one transport over it ([manual sync](backlog/manual-sync.md)).

### Fetching packages

- How a device learns which package a service runs and fetches its bytes from peers, and how it checks them against the version the service's ops carry. Getting a package this device lacks becomes fetching it from peers: the first step of a space's start, bounded by its startup timeout like the rest.

### Membership

- [Story 7](07-add-members.md), inviting and admitting a member: what a contact holds now that an identity is not one p2panda group id, its devices or a way to learn them. Admitting an identity means admitting the devices its assignment policy places in each service, and p2panda needs each device's key bundle before it can be added.

### Status

- Where `GET [@carbon]/status` reads access from, now that p2panda access is per device and CarbonDB speaks in identities. It already reports each service's lifecycle state; it also reports sync as off, waiting or on, and the peers this device has synced each service with.

## Build order

1. Fetching packages from peers.
2. The assignment policy in full, then inviting and admitting members (story 7).
3. Status and peers.

## Docs this changes

Once the assignment policy is settled, these passages, which describe an identity as a p2panda group of its devices, are rewritten for devices added directly: the Identity term in [README.md](README.md); Identity and devices, No secrets in replicated ops, Publishing an identity and Invitations in [implementation.md](implementation.md); the device calls in [docs/services.md](../docs/services.md); [story 2](02-add-device.md) and [story 7](07-add-members.md); and the [revoke device](backlog/revoke-device.md), [paper key](backlog/paper-key.md), [security review](backlog/security-review.md) and [public identity proofs](backlog/public-identity-proofs.md) backlog stories.
