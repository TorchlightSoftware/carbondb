# A person can initialize their identity on a new device

Status: implemented — single-node; the paper key, the public record of an identity's devices, secure secret storage and the replicated `[sync]` modes are not built

## Calls

```js
const carbon = await configure('./carbondb.toml')
const {id, device} = await carbon.call('POST [@carbon]/identity', {slug: 'alice', device: 'laptop', services: ['contacts']})
// this run and later ones
const identity = carbon.ref('identity:alice')   // the identity's own channel
const personal = identity.ref('personal:alice') // its personal channel
```

## Rules

- `configure` reads the TOML file once: where the system database is, its mode, and `[sync] mode`: `ref` (the default, syncing a channel the first time a ref reaches it, and from then on), `all`, `manual`, or `none` for single-node. Only `none` is built; `configure` refuses the others with a 501. No other call takes a directory.
- Creating an identity always creates its first device, with keys generated on this machine by `POST [@carbon]/identity`, an op in the system log, which never replicates. The secret key stays in the system database. `POST [identity]/origin` in the identity's own channel records the device's public key, name and tags (`manage`), and is what authorizes the key to act for the identity.
- Creating an identity creates two channels: its own, running `[identity]` and `[channel]`, and its personal channel, nested in it. The identity's own channel holds one ref, to the personal channel, and is sealed with `POST [channel]/seal` once founded, so its `[channel]` refuses every later POST, the same on every replica. Each of these steps is a call made as an effect of the `[@carbon]` op (see [docs/replication.md](../../docs/replication.md#effects-are-made-once)); the identity is recorded on this device only once all of them have applied, so it is listed only when whole.
- The identity's own channel is shared only by the person's own devices, never with anyone else, and nothing nests under it but the personal channel. Everything else of the person's nests under the personal channel.
- The personal channel runs the services `services` names, each of which this device must have registered (a 400 names those it has not). More can be added later with `POST [channel]/services` ([story 6](06-create-shared-channel.md)).
- The identity is private. The public record of its devices, `GET [@carbon]/identity/export` and `[identity] publish = true` in the TOML file, is deferred (see [the backlog story](../backlog/public-identity-proofs.md)); neither is built. What it would hold, now that `[identity]` keeps the devices' private names and tags beside their keys, is not yet defined.
- The paper key is returned once and never stored. Not built: the call returns none until recovery is designed (see [the backlog story](../backlog/paper-key.md)).
- Dev and test mode keep keys in SQLite in plaintext; production refuses to run without secure secret storage (designed later, not built).
- A display name is the app's profile data, not part of the identity.
- CarbonDB's system records are migrations in `crates/core/migrations/system/`, applied by the core.

## Acceptance

- After `POST [@carbon]/identity`, `carbon.ref('identity:alice')` in a fresh process reaches the same identity id and device.
- The id is the hash of `POST [identity]/origin` with the first device's public key, so every replica recomputes it and it stays the same as keys rotate; the device's private key never appears in any op or message.
- `GET [identity]/devices` on the identity's handle lists the first device with its name and tags.
- The identity's own channel refuses `POST [channel]/ref` after founding with a 403: it is sealed.
- A second identity on the same machine has different device keys from the first.
- Once the public record is built: it holds the identity's origin and its devices' public keys, never their names or tags.
