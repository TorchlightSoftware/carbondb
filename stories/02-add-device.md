# A person can add another device to their identity

Status: agreed

## Calls

```js
// on a device already in the identity, in its identity space: show a code
const identity = carbon.ref('identity:alice')
const {code} = await identity.call('POST [@carbon]/devices')
display(code)

// on the new device, in the system space: scan it, and join the identity
await carbon.call('POST [@carbon]/identity', {code: scannedCode, device: 'phone'})

// or the other way round: the existing device scans the new device's code
await identity.call('POST [@carbon]/devices', {code: scannedCode})
```

### Open questions

- How the new device shows its code when it is the one shown: which call in the system space generates its key pair and answers the code, before it knows the identity.
- How the side that showed its code learns the exchange finished: a watch, such as `GET [@carbon]/identities` on the new device, which needs watches on the system space, or `GET [device]/refs` on the existing one.
- Whether a call that waits on p2panda fits its service's timeout, or starts the work and a watch reports it done.

## Rules

- Either device may start, and either may show or scan; one scan completes the exchange. How the app offers this is up to its interface.
- The new device generates its keys in the system space, which never replicates, as part of `POST [@carbon]/identity` with a `code`. The existing device runs `POST [@carbon]/devices` in the identity space: it authorizes that key with `POST [device]/origin` there, and adds the new device to the identity's p2panda group, which gives it the group secrets of every space the identity is in. It records the new device's name and tags with `POST [device]/ref` in the personal space; without the `manage` tag, a device cannot write to `[space]`. The new device syncs the identity space and finds its spaces by walking the refs from it, through the personal space.
- Codes are single-use and short-lived.
- The identity's existing devices see every join through `GET [device]/refs` in the personal space; the interface decides whether and how to show it.

## Acceptance

- Two devices in one test process (two databases) complete a join in each direction; afterwards both list the same identity and both devices.
- A code used once, or after it expires, fails.
- The existing device can see the new device's key among the identity's devices (`GET [device]/devices`), and its name and tags among the personal space's device refs (`GET [device]/refs`).
- The identity's export gains the new device's public key, and not its name or tags.
