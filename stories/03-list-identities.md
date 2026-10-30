# A person can choose among the identities on this device

Status: agreed

## Calls

```js
const identities = await carbon.listIdentities()
// [{slug: 'brandon', id, device: 'laptop', lastUsed}, ...]
const identity = await carbon.resumeIdentity(identities[0].slug)
```

## Rules

- `configure` returns the `carbon` handle; identity calls are its methods, so no state lives in the module and a test can open two devices in one process.
- Entries carry slug, id, this device's slug in that identity, and when it was last used. The fields may evolve.

## Acceptance

- With two identities created on one database, `listIdentities` returns both, and resuming either returns the right one.
