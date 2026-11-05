# A person can initialize their identity on a new device

Status: agreed

## Calls

```js
const carbon = await configure('./carbondb.toml')
const {identity, personal, paperKey} = await carbon.createIdentity({slug: 'alice', device: 'laptop', services: ['contacts']})
// later runs
const identity = await carbon.resumeIdentity('alice')
```

## Rules

- `configure` reads the TOML file once: where the SQLite database is, its mode, and `[sync] mode`: `ref` (the default, syncing a space the first time a ref reaches it, and from then on), `all`, `manual`, or `none` for single-node. No other call takes a directory.
- Creating an identity always creates its device, with keys generated on this machine by `POST [@carbon]/identity` in system, which never replicates. `POST [device]/origin` in the identity space is what authorizes its key.
- Creating an identity also creates its personal space, running the services the call names, and the identity space holds a ref to it. The identity space holds only what may be published; the personal space holds the device's name and tags and everything else private.
- The identity is private by default. `GET [@carbon]/identity/export` returns it for the app to publish, and `[identity] publish = true` in the TOML file serves it to anyone who subscribes.
- The paper key is returned once and never stored.
- Dev and test mode keep keys in SQLite in plaintext; production refuses to run without secure secret storage (designed later).
- A display name is the app's profile data, not part of the identity.
- CarbonDB's system records are executable SQL in `schemas/`, applied by the core.

## Acceptance

- After `createIdentity`, `resumeIdentity('alice')` in a fresh process returns the same identity id and device.
- The id is the hash of the identity's `POST [identity]/origin` op, signed by the first device; the device's private key never appears in any op or message.
- A second identity on the same machine has different device keys from the first.
- The export of a new identity holds its origin and its device's public key, and not the device's name or tags.
