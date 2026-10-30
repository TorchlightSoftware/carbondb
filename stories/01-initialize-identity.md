# A person can initialize their identity on a new device

Status: agreed

## Calls

```js
const carbon = await configure('./carbondb.toml')
const {identity, paperKey} = await carbon.createIdentity({slug: 'brandon', device: 'laptop'})
// later runs
const identity = await carbon.resumeIdentity('brandon')
```

## Rules

- `configure` reads the TOML file once: where the SQLite database is and its mode. It holds no device settings, and no other call takes a directory.
- Creating an identity always creates its device, with keys generated on this machine.
- The paper key is returned once and never stored.
- Dev and test mode keep keys in SQLite in plaintext; production refuses to run without secure secret storage (designed later).
- The personal space is created separately (story 5); the app sequences onboarding as one step for the person.
- A display name is the app's profile data, not part of the identity.
- CarbonDB's system records are executable SQL in `schemas/`, applied by the core.

## Acceptance

- After `createIdentity`, `resumeIdentity('brandon')` in a fresh process returns the same identity id and device.
- The id is the hash of the identity's public key; the device's private key never appears in any op or message.
- A second identity on the same machine has different device keys from the first.
