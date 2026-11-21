# Channels

A channel names the services that work together, and holds the people given access to them. This page explains what a channel is, how channels are created and reach each other, who may change them, and the two channels every identity starts with. The calls themselves are listed in [services.md](services.md); how a service in a channel starts, and what a call waits for, is in [lifecycle.md](lifecycle.md).

## What a channel is

- **A channel** names its services, `[name]` to service, as candle-connect maps a name to a host in Otto, and holds the people given access to them. A call on a channel's handle resolves `[name]` through the channel. A service belongs to one channel, and a name the channel does not name gets a 404.
- **A service** is the code: a package, as an actor type or a Durable Object class is. Each service a channel names is one instance of it, with its own SQLite file, holding its tables and results, on every device that holds the channel (its store, `crates/core/src/channels/service.rs`); its ops sit in the channel's one log, beside every other service's. Many instances run the same package, such as a [chat] for a company and another for its marketing team; how many is a data custody decision made per use case.
- **`[channel]`** runs in every channel: its origin, the services it names, its refs, its members and invitations, and its manage list. A channel's id is the id of its `[channel]` service.
- **Names:** a channel is reached by its kind and a lowercase kebab-case slug, such as `identity:alice`, `company:foo-corp` and `team:sales`. **Kind** is a free label for apps and interfaces, `channel` by default; CarbonDB gives meaning only to `identity` and `personal`, the kinds it creates itself.
- **system** is the root: this device's own database, local and never replicated, holding what must be readable before any identity opens, such as the identities on this device and the services it holds. `carbon` is its handle, and only `[@carbon]` answers there.
- **`[@carbon]`** is the core's own service, marked `[@name]`, and runs in every channel like any other: its ops sit in the channel's log and replay on every replica, what it does once (creating a channel or a service's store, signing, enrolling a device) recorded as effects. It founds and lists identities, creates channels and services, enrolls devices, and reports the status and sync of this device's copy of a channel or service. What it changes in a channel it changes through that channel's own calls, made as effects of its own op. The system is a local channel, never replicated, whose `[@carbon]` founds and lists the identities on this device.
- **Calls between services:** a call to the same service runs inside the op, under a savepoint. A call to another service the channel names takes one of two bindings. Through `call` it is a message, the `call` effect: made on the op's first run, posted to that service as an op there, its answer recorded on the op's row, and every replay gets the recorded answer ([replication.md](replication.md#effects-are-made-once)); a write it posts stays when the op is later refused. Through `callsync`, a write is the same message bound to the op, which it follows ([causes](replication.md#causes)), and a read answers as of the op's place in the shared order on every run ([consistent reads](replication.md#consistent-reads)).

## Creating channels and services

Only the core creates spaces, as steps of `[@carbon]`'s own ops, which run once, on this device, and are never replayed. A new space's id is p2panda's when this device syncs, and derived from the op single-node; peers read it from the channel's own ops that name it.

1. `POST [@carbon]/channel` runs on the handle of the channel the new one nests in. It takes a `name`, shared with members, and an optional `kind`. The core creates the new channel's space, then `POST [channel]/ref` points to it from this channel, deriving a slug from the name, adding `-N` when that slug is taken among refs of its kind (an optional `slug` overrides it), and `POST [channel]/origin` writes its kind, name and manage list as its first op. The origin repeats the ref's fields because the channel's members may not be able to read the channel it was created from. It answers the ref.
2. `POST [@carbon]/services` adds a service: `[name]`, running `package`, a package registered on this device, which defaults to the name. The core creates its space, then `POST [channel]/services` records the entry: name, space, package and version, signed with the op's device key. `[channel]` names a service only with that signature, which only `[@carbon]` can make, since only the core holds device keys; every replica checks it against the op's device. A name the channel already uses, or `channel`, is refused with a 409; a package this device has not registered, with a 404 naming it. A manager may add services at any time.

When a `[@carbon]` op creating a channel or a service fails, the core deletes the spaces it created and stops following their p2panda spaces. Once it has succeeded, the ops naming them stay like any other.

## Rules on `[channel]`

- Every POST to `[channel]` but its origin requires `manage` on the channel, except `POST [channel]/leave`.
- An origin op can be written only once: `[channel]`'s once per channel, `[identity]`'s once per identity.
- `POST [channel]/seal` makes the channel refuse every later POST, the same on every replica.

## Reaching channels: refs and handles

- **Hierarchy lives in refs and manage lists.** A ref only points and grants nothing; a channel may have many. Attaching an existing channel to another, so that it is visible from there, is `POST [channel]/ref` on that other channel, which needs `manage` there.
- **Parenthood is set at creation.** A channel a person creates for themselves nests in their personal channel, so its ref stays private. An employee reaches a team channel through its company, because that access comes through the company; only a direct relationship, such as a consultant, puts a team ref straight in a personal channel.
- **`cascade`** on a ref takes `remove`, where losing access to the parent removes access to the child, and `delete`, where deleting the parent deletes the child, given manage there.
- **Handles and channels are one to one.** `carbon` is the system handle, and every channel is reached through refs, which also sets the identity a handle acts as:

```js
const sales = carbon.ref('identity:alice').ref('personal:alice').ref('company:foo-corp').ref('team:sales')
await sales.call('POST [chat]/messages', {text: 'lunch?'})
sales.channel   // the channel's id
```

`carbon.ref('identity:alice')` reaches the identity's own channel from the system database; every later `ref` reads the channel's `[channel]` refs as committed, at once, and throws a 404 for one that is not there. An app needs only `configure`, `services.add`, `ref`, `call` and `watch`; everything else is a call, and what a call creates is reached through `ref`. No state lives in the module, so a test can open two devices in one process. `[@carbon]` answers on `carbon` and on every channel's handle, about that channel.

## Access and manage

- **Access is per channel:** `pull`, `read`, `write` or `manage`. A member has it on every service in the channel, so every peer of a service holds the same channel; data that must stay partitioned goes in another channel. A device's access is the lower of what its identity is granted and what its own policy accepts.
- **`manage`** is CarbonDB's own permission, per channel, set at creation on the origin op and extended with `POST [channel]/manage`. It defaults to the calling identity. An empty list means no one may change the channel: add services or members, or delete it.
- Single-node, the creator is the only member, with `manage`. Adding members, invitations and leaving need peers ([story 7](../stories/07-add-members.md)).

## An identity's two channels

`POST [@carbon]/identity` creates two channels, like `/` and `/home`:

| Channel | Holds | Who reads it |
|---|---|---|
| `identity:alice` | `[channel]`: its origin and one ref, to the personal channel, sealed. `[identity]`: the identity's origin and its devices, with their keys, names and tags | The identity's own devices only |
| `personal:alice` | `[channel]`: refs to the channels the identity creates or joins, and the services it names; those services' personal data | The identity, plus anyone it adds |

- **The identity's own channel is never shared,** and nothing nests under it but the personal channel. It is founded as `POST [channel]/origin`, its ref to the personal channel, then `POST [channel]/seal`, after which its `[channel]` refuses every later POST.
- **Creating an identity creates its first device,** with keys generated on this machine by `POST [@carbon]/identity`, an op in the system log. The secret key stays in the system database and never appears in an op. `POST [identity]/origin` records the device's public key, name and tags (`manage`), and authorizes the key to act for the identity. The identity's id is the hash of that origin with the first device's public key, so every replica recomputes it and it stays the same as keys rotate. A second identity on the same machine has device keys of its own, so the two cannot be linked.
- **Founding is one `[@carbon]` op:** the core creates the spaces of the own channel, its `[identity]`, the personal channel and its services, then sets each up by a call made as an effect of that op; the identity is recorded on this device only once all of them have applied, so it is listed only when whole, and if any fails, the spaces it created are deleted.
- **The personal channel** is where everything else of the person's nests: refs to the channels they create or join, and personal data. It runs `[channel]` and the services `POST [@carbon]/identity` names in `services`, each registered on this device (a 400 names those that are not), and more added later with `POST [@carbon]/services`. Others may join it later, such as a caretaker or someone holding power of attorney, without being conflated with the identity.
- **Both channels' manage lists** name the identity itself, whose id is known before its origin is written.
- **A display name** is the app's profile data, not part of the identity.
- **CarbonDB's system records** are migrations in `crates/core/migrations/system/`, applied by the core.

## Not built yet

- Adding a device to an identity ([story 2](../stories/02-add-device.md)), a member's new device joining their channels ([backlog](../stories/backlog/member-devices.md)), and revoking a device ([backlog](../stories/backlog/revoke-device.md)).
- Members, invitations, removing a member and leaving ([story 7](../stories/07-add-members.md), [backlog](../stories/backlog/remove-member.md)).
- The paper key ([backlog](../stories/backlog/paper-key.md)) and the public record of an identity's devices ([backlog](../stories/backlog/public-identity-proofs.md)).
- Secure secret storage: dev and test keep keys in SQLite in plaintext, and production is meant to refuse to run without it.
- Moving a channel to another parent ([backlog](../stories/backlog/reassign-space-parent.md)), and a parent's rules on its children's members ([backlog](../stories/backlog/parent-rules-on-child-members.md)).
- Removing a service from a channel, deleting a channel, and moving a service to a new version ([backlog](../stories/backlog/upgrade-package-version.md)).
