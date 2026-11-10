# A person can choose among the identities on this device

Status: implemented — single-node

## Calls

```js
const identities = await carbon.call('GET [@carbon]/identities')
// [{slug: 'alice', id, space, personal, device}, ...]
const identity = carbon.ref(`identity:${identities[0].slug}`)
```

## Rules

- `configure` returns the `carbon` handle; identity calls are made on it, so no state lives in the module and a test can open two devices in one process.
- Entries carry the slug, the identity id, its identity and personal spaces, and this device's public key in that identity, never its secret key. The fields may evolve, such as adding when it was last used.

## Acceptance

- With two identities created on one database, `GET [@carbon]/identities` returns both, and a ref to either reaches the right one.
