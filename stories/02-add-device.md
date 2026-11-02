# A person can add another device to their identity

Status: agreed

## Calls

```js
// on the new device
const join = await carbon.joinIdentity({device: {name: 'phone', type: 'mobile'}})
// or on a device already in the identity
const join = identity.addDevice()

display(join.qr)            // either side may show its code
join.apply(scannedCode)     // either side may scan the other's
const result = await join.done
```

## Rules

- Either device may start, and either may show or scan; one scan completes the exchange. How the app offers this is up to its interface.
- The new device generates its keys with `POST [@carbon]/devices` in system, which never replicates. The existing device authorizes that key with `POST [device]/origin` in the identity space, and adds the new device to the identity's p2panda group, which gives it the group secrets of every space the identity is in. It records the new device's name and tags with `POST [device]/ref` in the personal space; without the `manage` tag, a device cannot write to `[space]`. The new device syncs the identity space and finds its spaces by walking the refs from it, through the personal space.
- Codes are single-use and short-lived.
- The identity's existing devices see every join through `GET [device]/refs` in the personal space; the interface decides whether and how to show it.

## Acceptance

- Two devices in one test process (two databases) complete a join in each direction; afterwards both list the same identity and both devices.
- A code used once, or after it expires, fails.
- The existing device can see the new device's key among the identity's devices (`GET [device]/devices`), and its name and tags among the personal space's device refs (`GET [device]/refs`).
- The identity's export gains the new device's public key, and not its name or tags.
