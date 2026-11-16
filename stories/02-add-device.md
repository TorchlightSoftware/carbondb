# A person can add another device to their identity

Status: agreed — needs peers until p2panda; single-node, `POST [identity]/devices` fails with `503 UNREACHABLE` and `POST [@carbon]/identity` with a `code` with `501 SINGLE_NODE`

## Calls

```js
// on a device already in the identity, on its own channel: show a code
const identity = carbon.ref('identity:alice')
const {code} = await identity.call('POST [identity]/devices', {name: 'phone', tags: []})
display(code)

// on the new device, on the system handle: scan it, and join the identity
await carbon.call('POST [@carbon]/identity', {slug: 'alice', code: scannedCode, device: 'phone'})

// or the other way round: the existing device scans the new device's code
await identity.call('POST [identity]/devices', {code: scannedCode, name: 'phone', tags: []})
// {pubkey, name: 'phone', tags: []}

// later, from any of the identity's devices
await identity.call(`PUT [identity]/devices/${pubkey}`, {name: 'work phone', tags: ['manage']})
```

### Open questions

- How the new device shows its code when it is the one shown: which call on the system handle generates its key pair and answers the code, before it knows the identity.
- How the side that showed its code learns the exchange finished: a watch, such as `GET [@carbon]/identities` on the new device, which needs watches on the system handle, or `GET [identity]/devices` on the existing one. When the existing device showed the code, which op records the new device's key, name and tags once the exchange finishes is not yet defined; scanning the other way, `POST [identity]/devices` with the `code` records it in one op.
- Whether a call that waits on p2panda fits its service's timeout, or starts the work and a watch reports it done.

## Rules

- Either device may start, and either may show or scan; one scan completes the exchange. How the app offers this is up to its interface.
- The new device generates its keys on the system handle, which never replicates, as part of `POST [@carbon]/identity` with a `code`. The existing device runs `POST [identity]/devices` in the identity's own channel: the handshake with the new device is an effect of that op, made once on its first run and recorded (see [docs/replication.md](../docs/replication.md#effects-are-made-once)), and the op records the device's public key, name and tags in `[identity]`. Only the identity itself adds, renames or retags its devices.
- Enrolling adds the new device to every service of the identity's own channel, its `[channel]` included, and of its personal channel. That is a p2panda membership change, not a `[channel]` op, so the seal on the identity's own channel does not stop it. For now CarbonDB adds the device to each service's space directly (see [upstream group write](backlog/upstream-group-write.md)).
- The new device syncs the identity's own channel and finds the rest by walking refs from it, through the personal channel. CarbonDB's core reads the channels its identity belongs to, joins their services and fetches them, at the lower of the identity's grant and the device's own policy. Where a person sets a device's policy is not yet defined.
- Channels shared with other people need a device with `manage` there to add the new device; for now one must be online and is presented with the request (see [member devices](backlog/member-devices.md)).
- Codes are single-use and short-lived.
- The identity's existing devices see every join through `GET [identity]/devices`; the interface decides whether and how to show it.

## Acceptance

- Two devices in one test process (two databases) complete a join in each direction; afterwards both list the same identity and both devices.
- A code used once, or after it expires, fails.
- The existing device sees the new device's key, name and tags in `GET [identity]/devices`.
- Once the public record of an identity's devices is built ([backlog](backlog/public-identity-proofs.md)): it gains the new device's public key, and not its name or tags.
