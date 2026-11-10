# A person can initialize their identity on a new device

Status: implemented — single-node; the paper key, export, publishing, secure secret storage and the replicated `[sync]` modes are not built

## Calls

```js
const carbon = await configure('./carbondb.toml')
const {id, device} = await carbon.call('POST [@carbon]/identity', {slug: 'alice', device: 'laptop', services: ['contacts']})
// this run and later ones
const identity = carbon.ref('identity:alice')
const personal = identity.ref('personal:alice')
```

## Rules

- `configure` reads the TOML file once: where the SQLite database is, its mode, and `[sync] mode`: `ref` (the default, syncing a space the first time a ref reaches it, and from then on), `all`, `manual`, or `none` for single-node. Only `none` is built; `configure` refuses the others with a 501. No other call takes a directory.
- Creating an identity always creates its device, with keys generated on this machine by `POST [@carbon]/identity` in the system space, which never replicates. `POST [device]/origin` in the identity space is what authorizes its key.
- Creating an identity also creates its personal space, running the services the call names, and the identity space holds a ref to it. The identity space holds only what may be published; the personal space holds the device's name and tags and everything else private.
- The identity is private by default. `GET [@carbon]/identity/export` returns it for the app to publish, and `[identity] publish = true` in the TOML file serves it to anyone who subscribes. Neither is built.
- The paper key is returned once and never stored. Not built: the call returns none until recovery is designed (see [the backlog story](backlog/paper-key.md)).
- Dev and test mode keep keys in SQLite in plaintext; production refuses to run without secure secret storage (designed later, not built).
- A display name is the app's profile data, not part of the identity.
- CarbonDB's system records are migrations in `crates/core/migrations/system/`, applied by the core.

## Acceptance

- After `POST [@carbon]/identity`, `carbon.ref('identity:alice')` in a fresh process reaches the same identity id and device.
- The id is the hash of the identity's `POST [identity]/origin` op, signed by the first device; the device's private key never appears in any op or message.
- A second identity on the same machine has different device keys from the first.
- Once export is built: the export of a new identity holds its origin and its device's public key, and not the device's name or tags.
