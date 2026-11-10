# A person can choose among the identities on this device

Status: agreed

## Calls

```js
const identities = await carbon.call('GET [@carbon]/identities')
// [{slug: 'alice', id, device: 'laptop', lastUsed}, ...]
const identity = carbon.ref(`identity:${identities[0].slug}`)
```

## Rules

- `configure` returns the `carbon` handle; identity calls are made on it, so no state lives in the module and a test can open two devices in one process.
- Entries carry slug, id, this device's slug in that identity, and when it was last used. The fields may evolve.

## Acceptance

- With two identities created on one database, `GET [@carbon]/identities` returns both, and a ref to either reaches the right one.
