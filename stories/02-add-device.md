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
- The existing device signs the new one into the identity's log and sends it the per-identity key, encrypted to the new device's keys.
- Codes are single-use and short-lived.
- CarbonDB announces every join to the identity's existing devices; the interface decides whether and how to show it.

## Acceptance

- Two devices in one test process (two databases) complete a join in each direction; afterwards both list the same identity and both devices.
- A code used once, or after it expires, fails.
- The existing device can see the new device among the identity's devices (`GET [carbon]/devices`).
