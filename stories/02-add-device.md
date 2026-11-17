# A person can add another device to their identity

Status: agreed — the direction where the new device shows its code is built ([docs/sync.md](../docs/sync.md#enrolling-a-device)), tested with two devices in one process; the reverse direction, where the existing device shows a code, is not. Single-node, `POST [identity]/devices` fails with `503 UNREACHABLE`, and `POST [@carbon]/devices` and `POST [@carbon]/identity` with a `code` with `501 SINGLE_NODE`

## Calls

```js
// on a device already in the identity, on its own channel: show a code
const identity = carbon.ref('identity:alice')
const {code} = await identity.call('POST [identity]/devices', {name: 'phone', tags: []})
display(code)

// on the new device, on the system handle: scan it, and join the identity
await carbon.call('POST [@carbon]/identity', {slug: 'alice', code: scannedCode, device: 'phone'})

// or the other way round (built): the new device shows a code on its system handle
const {code: shown} = await carbon.call('POST [@carbon]/devices')
display(shown)
// and the existing device scans it
await identity.call('POST [identity]/devices', {code: scannedCode, name: 'phone', tags: []})
// {pubkey, name: 'phone', tags: []}

// later, from any of the identity's devices
await identity.call(`PUT [identity]/devices/${pubkey}`, {name: 'work phone', tags: ['manage']})
```

### Open questions

- When the existing device showed the code, which op records the new device's key, name and tags once the exchange finishes is not yet defined; scanning the other way, `POST [identity]/devices` with the `code` records it in one op.

## Rules

- Either device may start, and either may show or scan; one scan completes the exchange. How the app offers this is up to its interface. The first milestone in [p2panda-sync.md](p2panda-sync.md) builds the new device showing its code and the existing device scanning it; the reverse follows.
- The new device generates its keys on the system handle, which never replicates: as part of `POST [@carbon]/identity` with a `code` when it scans, or in a `[@carbon]` call that answers its code when it is the one shown. The existing device runs `POST [identity]/devices` in the identity's own channel: the handshake with the new device is an effect of that op, made once on its first run and recorded (see [docs/replication.md](../docs/replication.md#effects-are-made-once)), and the op records the device's public key, name and tags in `[identity]`. Only the identity itself adds, renames or retags its devices.
- Enrolling adds the new device to every service of the identity's own channel, its `[channel]` included, and of its personal channel, at `manage` under the default assignment policy; the existing device makes those changes in the enroll effect. That is a p2panda membership change, not a `[channel]` op, so the seal on the identity's own channel does not stop it. For now CarbonDB adds the device to each service's space directly (see [upstream group write](backlog/upstream-group-write.md)).
- The new device loads the identity's own channel, and in the `ref` sync mode finds the rest by watching the refs and services of each channel it holds, from the identity's own channel through the personal channel, at the lower of the identity's grant and the device's own policy ([implementation.md](implementation.md#sync-status-and-partitions)). Where a person sets a device's policy is not yet defined.
- Channels shared with other people need a device with `manage` there to add the new device; for now one must be online and is presented with the request (see [member devices](backlog/member-devices.md)).
- When the new device shows its code, the code carries its p2panda `Member` bytes (its public key and key bundle, cross-signed), a pairing secret and its relay URL if any, so the existing device's op records it and can add it without waiting on it. The enroll effect answers at once and hands the network work to p2panda, which adds the device and sends it the identity and its own channel's space id over the pairing topic; the device that made the enroll op adds the new device wherever it is missing, at start and whenever `[identity]` changes, so a crash loses nothing. The new device learns it finished when that payload arrives, and the app sees it in `GET [@carbon]/identities` ([docs/sync.md](../docs/sync.md#enrolling-a-device)).
- Codes are single-use and expire after 10 minutes.
- The identity's existing devices see every join through `GET [identity]/devices`; the interface decides whether and how to show it.

## Acceptance

- Two devices in one test process (two databases) complete a join in each direction; afterwards both list the same identity and both devices.
- A code used once, or after it expires, fails.
- The existing device sees the new device's key, name and tags in `GET [identity]/devices`.
- Once the public record of an identity's devices is built ([backlog](backlog/public-identity-proofs.md)): it gains the new device's public key, and not its name or tags.
