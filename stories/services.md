# Builtin services

CarbonDB ships four services. Their APIs are provisional and follow `stories/oplog-examples.md`, which shows them in use.

| Service | Replicated | Runs in | Holds |
|---|---|---|---|
| `[@carbon]` | No | The system space and every space, in that space's context | Whatever must happen once, on one device |
| `[space]` | Yes | Every replicated space | What the space is, what it points to, and who belongs |
| `[identity]` | Yes | Identity spaces | The identity's origin |
| `[device]` | Yes | Identity and personal spaces | Device public keys (origin) in the identity space; device names and tags (refs) in the personal space |

Most services are replicated, which is what creates our P2P collaboration capabilities.  `@carbon`, or any other service that you would see prepended by `@` is a local, non replicated service.  This is useful for making system calls or anything else that needs to run local only: actions that are private and not to be repeated for each peer.  Replicated services can call `[space]` when they have the access to do so, and can never call `[@carbon]`.

CarbonDB's `[identity]` is distinct from an app's own service that may share the name; they should be able to operate side by side, with an app's [identity] offering RBAC for organization spaces, and CarbonDB's [identity] offering P2P personal identity management.  It doesn't seem like they should ever be operating in the same space, so naming conflict is not an issue for now?

## [@carbon]

The local service for this device's own CarbonDB. Its calls are kept in the op log of the space they ran in, marked with this device, or in the system log for calls on the system handle, so an interrupted call can be finished after a crash (in theory). What it writes in replicated spaces goes through `[space]`, `[identity]` and `[device]`: in the space it runs in, as sub-calls carried on its own log row, which other devices replay without it; in another space, as ops there. The acting identity comes from the handle (current space).

| Call | Called from | Input | What it does |
|---|---|---|---|
| `POST [@carbon]/identity` | The system space | `slug`, `device`, `services` (the personal space's), `code` (to join) | Without a `code`, founds an identity: generates the first device's key pair. Creates the identity space and writes its `POST [space]/origin`, `POST [identity]/origin` and `POST [device]/origin`. Creates the personal space, writes its `POST [space]/origin` and the first device's `POST [device]/ref`, and writes a `POST [space]/ref` to it in the identity space. With a `code` scanned from a device already in the identity, joins it instead: generates this device's key pair and keeps it here, with the identity, once that device has authorized it |
| `GET [@carbon]/identities` | The system space | | The identities on this device, each with its slug, id, identity space, personal space and this device's public key |
| `GET [@carbon]/identity/export` | The identity space | | The identity space's ops with their CarbonDB signatures, for the app to publish where it likes |
| `POST [@carbon]/devices` | The identity space | `code` (optional) | Adds a device to the identity. With a `code` scanned from the new device, authorizes its key with `POST [device]/origin`, adds it to the identity's p2panda group and records its name and tags with `POST [device]/ref` in the personal space. Without one, answers a code for the new device to scan |
| `POST [@carbon]/space` | The space to nest it in | `kind`, `name`, `services`, `manage` (default: the calling identity), `cascade` | Has the calling space accept `POST [space]/ref`, creates the space in p2panda, and writes its `POST [space]/origin` |
| `POST [@carbon]/join` | The space that reaches it | `code` | Subscribes to the space, announcing this device's key bundle; writes `POST [space]/ref` in the calling space unless it already holds one |
| `POST [@carbon]/members` | The space being changed | `contact`, `access` (default `write`) | Returns an invite code. Once the member joins, a device with manage adds them in p2panda and writes `POST [space]/members` |
| `POST [@carbon]/manage` | The space being changed | `add` | Grants manage in p2panda to the entry's devices and writes `POST [space]/manage` |
| `GET [@carbon]/members` | The space being read | | The space's members and access levels, read from p2panda |
| `GET [@carbon]/manage` | The space being read | | The space's manage list |
| `GET [@carbon]/status` | The space being read | | The space's metadata and its status on this device |
| `POST [@carbon]/sync/start` | The space being synced | | Starts syncing the space |
| `POST [@carbon]/sync/stop` | The space being synced | | Stops syncing and keeps the data |
| `DELETE [@carbon]/data` | The space being cleared | | Deletes this device's data for the space. p2panda has no per-space delete, so its store keeps the encrypted ops |

Each call is answered only in the space its row names. Made anywhere else, it is refused with a 404 that says where it is answered.

`GET [@carbon]/status` never waits on the network:

```js
{space, kind, name, services, manage, access: ['write'], sync: 'on', peers: [{device, lastSynced}]}
```

- `access` mirrors p2panda's levels. `[]` means no access, and it is `undefined` until the space has synced. During a partition it holds the last known access.
- `sync` is `off`, `waiting` (subscribed, nothing has arrived yet) or `on`.
- `peers` holds the peers this device has synced this space with, and when. It cannot know about peers it has never met.

During a partition, GETs read the local copy, and writes apply locally and publish later; if the member was removed meanwhile, the removal cutoff discards them. A call on a `waiting` space is rejected.

## [space]

Every replicated space runs this service implicitly. Every POST to it requires manage on the space. A second `POST [space]/origin` is rejected.

| Call | Input | What it records |
|---|---|---|
| `POST [space]/origin` | `kind`, `name`, `services`, `manage` | The space's first op: what the space is. The services it names fix what is in scope for the space |
| `POST [space]/ref` | `space`, `kind`, `name`, `cascade` | A pointer to another space. It grants nothing |
| `GET [space]/refs` | | The spaces this space points to |
| `GET [space]/ref` | `kind`, `slug` | The ref to one space, by its kind and slug |
| `GET [space]/origin` | | The space's origin: its kind, name, services and manage list |
| `GET [space]/manage` | | The space's manage list |
| `POST [space]/members` | `identity` | A member joining, as an ordered event for triggers |
| `POST [space]/trigger` | `to`, `in`, `out` | Designed, not built. A trigger: when a call to `to` succeeds, its input is forwarded to each key in `in` and its output to each key in `out`, in the applying transaction on every replica |
| `POST [space]/manage` | `add` | A change to the manage list |

`cascade` on a ref takes `remove`, where losing access to this space removes access to the space it points to, and `delete`, where deleting this space deletes that one, given manage rights there.

## [identity]

| Call | Input | What it records |
|---|---|---|
| `POST [identity]/origin` | `device` | The identity's founding entry, naming the first device's public key. Its hash is the identity's id, and a second one is rejected |
| `GET [identity]/origin` | | The identity's id and its first device's public key |

## [device]

| Call | Space | Input | What it records |
|---|---|---|---|
| `POST [device]/origin` | Identity | `pubkey` | The authority for a device's key to act for the identity. An existing device writes it for a new one, and a second one for the same public key is rejected |
| `GET [device]/devices` | Identity | | The public keys authorized to act for the identity |
| `POST [device]/ref` | Personal | `pubkey`, `name`, `tags` | A device's name and tags, which stay private. A second ref for the same public key replaces the first |
| `GET [device]/refs` | Personal | | The identity's devices, with names and tags |

As with `[space]`, the origin is written where the thing lives and the ref where it is pointed to from.

## Not yet defined

- Removing a member, including what carries the removal cutoffs.
- Leaving and deleting a space.
- Revoking a device.
