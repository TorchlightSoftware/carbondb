# Design goals: CarbonDB as a runtime for services

CarbonDB lets a service written in the shape of an ordinary backend service (named calls, SQL tables, handlers) run on every member's device, with the replication protocol taking the place of HTTP. This note records who is responsible for what, how an op is applied, and a first sketch of the API. [implementation.md](implementation.md) holds the full agreed design, [services.md](services.md) the builtin calls, [packages.md](packages.md) how services are packaged and versioned, [oplog-examples.md](oplog-examples.md) worked examples, and [docs/replication.md](../../docs/replication.md) how replicas order, rewind and replay ops and their effects.

## The promise to a service author

- **Ops are service calls.** A write is a call such as `POST [chat]/messages {text}`; CarbonDB carries it to every member of the channel the service runs in.
- **You define your calls.** A service declares its calls and MAY tag them with metadata to assist in merging state (for example, that a POST only appends). Ordering needs are derived from the HTTP method first: GET is never an op, PUT and DELETE are idempotent and scoped to the resource their path names, POST and PATCH promise nothing unless tagged.
- **CarbonDB orders, merges and rewinds.** Every replica of a service applies the same ops in the same order, re-running the service's own handler, so business rules give the same answer everywhere. When a late op arrives, CarbonDB rewinds only as far as it must, and never beyond the service.
- **What you do outside your tables happens once.** A call to another service, or any other effect, is made on the op's first run and recorded with the op; every replay gets the recorded answer.
- **You can react to state changes.** `watch` re-runs a GET whenever a committed op writes a table it read, for interfaces.
- **Replication is the transport.** Every mutation goes through replication. Handlers are called by the replication protocol rather than by HTTP. The author's own device applies the call first, through the same path every other replica uses.

## Domain Terms

- **Identity**: a person, logically. A record of the device keys added to it and revoked from it, each entry signed by an existing device (Keybase's model). The record is `[identity]`, in the identity's own channel, which runs only `[identity]` (and `[channel]` for its one ref, to the personal channel), is shared only by the person's own devices, and is sealed at founding. Permissions and custody attach to the identity. Its id is the hash of its `POST [identity]/origin` with the first device's public key, so it stays the same as keys rotate; its slug is a local pet name, unique on this machine. The public record of its devices is deferred ([backlog](../backlog/public-identity-proofs.md)).
- **Device**: a physical machine enrolled in one identity. It generates its own keys, which never leave it; it signs, and authorizes subkeys. One machine holding several identities has separate device keys for each, so they cannot be linked. Its metadata (name, tags) is self-declared and kept privately in `[identity]`; no fingerprinting. Its own policy caps the access it takes on, so a pull-only device is a blind relay. A paper key is an offline device.
- **Personal channel**: where everything else of the person's nests, created with the identity, like `/home` beside `/`. It holds refs to the channels the identity creates or joins, and personal data. Others, such as a caretaker or someone holding power of attorney, may join it without being conflated with the identity.
- **Channel**: a group of services, each named by a pet name such as `[chat]`, and the people given access to them. A call on a channel's handle resolves `[name]` through the channel, as candle-connect maps a name to a host. Channels nest through refs. A channel is not a tenant: a tenant holds many channels.
- **Service**: a module of tables and calls keyed by service key (`POST [chat]/messages`), shipped as a package with its `service.toml`. Its version is the hash of its package. Each service a channel names is one instance of a package, with its own op log and SQLite file, in a space of its own; many instances may run the same package. A service belongs to one channel.
- **Access**: `pull`, `read`, `write` or `manage`, given per channel; a member has it on every service in the channel. A device takes the lower of its identity's grant and its own policy.
- **Registered services**: the packages an app registers in `main()` on every start with `carbon.services.add(name, impl)`. A device applies ops only for services whose package it has registered.
- **system**: the root, this device's local database and op log, holding what must be readable before any identity opens, such as the identities on this device and the spaces it holds. It is never replicated. Every channel is reached from it through refs, and `carbon` is its handle. Each channel has one handle.
- **Op**: a service call carried in a service's log, applied by every replica of the service in one shared order. An op on a local service runs only on the device that made it, and is kept in the system log; what it changes in channels it changes by calls made as effects.
- **Effect**: what a handler does outside its own service's tables, such as calling another service, creating a channel or service, or a p2panda membership change. Made on the op's first run and recorded on its log row; replays get the recorded output. A service declares its effects in `service.toml`, `replicate = "never"` (the default) or `"always"`. Effects that need peers fail as unreachable until p2panda.
- **Local service**: a service marked `[@name]`, running only on this device. Replicated services can never call it.
- **Origin op**: a channel's first op, `POST [channel]/origin`, carrying what the channel is: its kind, name and manage list. An identity's `POST [identity]/origin` names its first device.
- **Ref**: a pointer from a channel to another, written with `POST [channel]/ref`, or by creating or joining a channel. It grants nothing; access does. A channel has many refs, and a device finds channels by walking them from its identity's own channel, through its personal channel.
- **Manage**: CarbonDB's own permission, per channel, listed on the origin op. CarbonDB maps it onto p2panda's Manage, which p2panda grants only to individual devices. With an empty list, no one may change the channel, and it dissolves when all its members leave.
- **[@carbon]**: the local service for this device's own CarbonDB, answering only on `carbon`. It does what must happen once, on one device, with its keys or settings: founding and listing identities, and the status, sync and deletion of this device's copy of a channel or service.
- **[channel]**: the core service running in every channel's own space, holding its origin, its services, refs, members, invitations and manage list, and the seal.
- **[identity]**: CarbonDB's own service for an identity's origin and its devices, with their names and tags, running only in an identity's own channel. An app may name its own service `[identity]`, such as one holding roles, in another channel, since names resolve per channel; it registers the package under another name.

## Responsibilities

### CarbonDB core (Rust, loaded into Node through napi-rs)

- **Networking and membership, from p2panda:** the node, each service's space and its members.
- **Identity:** the device constellation (enroll a device by QR code, revoke one, recover with a paper key) and the blind relay, per the README.
- **The op envelope:** device (signs and syncs), author (the person, for access control and audit), hybrid logical clock time, the version (the hash of the package that handles it), and the call itself as a body the core does not interpret beyond its service key.
- **The shared order:** a service's ops sorted by (time, device, op hash); duplicates dropped.
- **SQLite:** one database per service, holding its tables beside its op log and results, and the calls a handler uses to read and write it.
- **Undo:** each op's writes recorded as a SQLite session changeset; rewinding applies the inverse changesets of the ops a late op must re-run, newest first.
- **The apply-and-rewind loop:** the core owns the order and the changesets, so it decides when a handler runs or re-runs, and calls the service runtime to run it.
- **Effects:** the registry of what this device can do outside a service's tables, each effect made on an op's first run, recorded on its row, and replayed after.
- **Channels:** resolving `[name]` to the service the channel names.
- **Watches:** the core keeps each watch's read set and re-runs its GET after every commit that wrote a table it read, and a write answers its caller once the watches its commit owes a run have run. Apps get no separate event API.
- **Builtin services:** `[@carbon]`, `[channel]` and `[identity]`, implemented in Rust.

### Repeatability of State Mutations

Every state change would fall into one of three classes:

┌────────────────┬──────────────────────────┬───────────────────────┐
│     Class      │         Example          │         Rule          │
├────────────────┼──────────────────────────┼───────────────────────┤
│ Undoable       │ SQLite writes in the     │ Undone and re-run by  │
│                │ service's own tables     │ rewind                │
├────────────────┼──────────────────────────┼───────────────────────┤
│ Repeatable,    │ Content-addressed blob   │ Safe in any order, no │
│ grow-only      │ puts                     │  undo needed; the op  │
│                │                          │ carries the hash      │
├────────────────┼──────────────────────────┼───────────────────────┤
│                │ Calling another service, │ An effect: made once  │
│ Irreversible   │ p2panda removal,         │ on the op's first     │
│                │ creating a channel,      │ run, recorded, and    │
│                │ sending email            │ replayed after        │
└────────────────┴──────────────────────────┴───────────────────────┘

Deleting a blob is the CALM case: it retracts a fact, so it should be garbage collection driven by settled state, not something a handler does. The email row is the weak spot, because a rewind can't recall an email once sent: as an effect it is sent once, and a rewind replays its recorded output. Creating a channel or a service is irreversible too, so it is an effect, made once, on one device.

TODO: expand this as a set of rules that services need to follow in order to ensure eventual consistency.

### Service runtime (JavaScript)

The core routes every call to a service by the name its channel gives it and hands it to the service's entry point. For an npm service, `carbondb` is that entry point:

- **Routing:** a service key to its handler, within the service. Validating input is the service's own business.
- **The handler context:** the call's input and path parameters, its `sql` binding under `env`, `call` for calls of its own and `effect` for other effects, and the op, with its author and time, and ids and randomness drawn from it so every replica computes the same values.
- **Schema:** the service's own `init` sets up its tables, for example by applying its migrations.
- **Access:** whether this author may make this call. Not built: today a service checks it in its handlers.
- **Later:** richer service runtimes, such as one that derives schema defaults from the op and builds queries from data, run on the same core.

### What the CarbonDB core never knows

The schemas, routes, validation, roles and merge strategy of app services, beyond the shared order. A service is ops in and SQLite writes and effects out; what happens in between is the service's business.

## Applying an op

1. The core picks the next op in the service's op log and opens a transaction and a session on the service's database.
2. It routes the op to the service's entry point, with a handler context. Handlers may be async; the core waits for the context to settle before it continues.
3. The handler reads and writes through its `sql` binding. Those calls are synchronous and run inside the open transaction. Its effects, such as a call to another service, are made on the op's first run, and replayed from the op's row on every later run.
4. When the context settles with a value, the core commits and records the op's result as applied with its effects, and the session's changeset says which tables it wrote. When it settles with an error, or not within its service's timeout, the core rolls back and records the result as refused, with its error. Each changeset is kept with the result for undo.
5. One op at a time per service, since the transaction stays open across the handler's awaits; services apply side by side.

## Undo, write sets and read sets

- **Session extension (the target for undo).** SQLite records the rows an op wrote, with old and new values. That changeset is both the undo record (rusqlite has `Changeset::invert`) and the op's write set. rusqlite is pinned to 0.39.0, the newest version that links beside p2panda's sqlx. That has compiled and run in a scratch crate, but not yet beside a live p2panda node.
- **Authorizer (read sets in the core).** SQLite reports every table and column a statement reads as it is prepared. That gives each op's read set at table and column level, without knowing any schema. It cannot see which rows a WHERE clause matched, so the core finds the one row a statement looks up by its whole key, confirmed by SQLite's query planner; see [reading one row](../../docs/replication.md#reading-one-row-by-its-key). It also refuses a service's statement that names CarbonDB's `carbon_` tables or reaches another file.
- **Why read sets.** When a late op arrives, an op after it must re-run if it read anything the late op wrote. Write sets alone cannot tell that.

## Stories

1. [A person can initialize their identity on a new device](01-initialize-identity.md)
2. [A person can add another device to their identity](02-add-device.md)
3. [A person can choose among the identities on this device](03-list-identities.md)
4. [An app developer can register the services the app runs](04-register-services.md)
5. [A person can call a service in their personal channel](05-call-personal-channel.md)
6. [A person can create a shared channel naming its services](06-create-shared-channel.md)
7. [A person can add members to a shared channel](07-add-members.md)
8. [A person can call a service in a shared channel](08-call-shared-channel.md)
9. [A person can see changes in a channel as they arrive](09-watch-channel.md)

### Backlog

- [A person can revoke a lost device](../backlog/revoke-device.md)
- [A manager can remove a member, and a member can leave or delete a channel](../backlog/remove-member.md)
- A person grants remote access with roles without membership
- [A person learns a call was rejected after merge](../backlog/rejected-after-merge.md)
- [Old ops are refused or re-run safely across versions and time](../backlog/old-ops.md)
- CarbonDB identity carries app-defined vouchers
- [A person can host services on heavy devices and reach them from light ones](../backlog/heavy-and-light-devices.md)
- [CarbonDB gets a security and privacy review after the prototype](../backlog/security-review.md)
- [Replicas are proven to hold the same ops when a member is removed](../backlog/verify-replica-op-sets.md)
- [A manager can move a channel to a different parent](../backlog/reassign-space-parent.md)
- [A company can allow or prevent adding a member to its team](../backlog/parent-rules-on-child-members.md)
- [Anyone can verify an identity's devices and public proofs](../backlog/public-identity-proofs.md)
- [A manager can move a service to a new version](../backlog/upgrade-package-version.md)
- [A service can store files](../backlog/file-bindings.md)
- [An app developer can install carbondb on any desktop platform](../backlog/native-packaging-per-platform.md)
- [An iOS user runs the app's shipped services at native speed](../backlog/ios-ahead-of-time-packages.md)
- [An app developer's npm service runs at the version npm installed](../backlog/npm-package-integrity.md)
- [An app developer can run a Worker's D1 queries on CarbonDB unchanged](../backlog/d1-compatibility.md)
- [A person's interrupted call finishes when the app restarts after a crash](../backlog/resume-after-crash.md)
- [A person gets a paper key when they create an identity](../backlog/paper-key.md)
- [An app developer registers a service by its package or crate name](../backlog/package-names.md)
- [The core keeps its own timers once replication runs](../backlog/core-timers.md)
- [An app can sign a message as an identity and verify one](../backlog/sign-and-verify.md)
- [p2panda lets a device write through a group it belongs to](backlog/upstream-group-write.md)
- [A member's new device joins their channels](backlog/member-devices.md)
- [An identity can replace the default assignment policy](../backlog/assignment-policy-override.md)

Adding a service to an existing channel, once [backlog](../backlog/add-services-to-space.md), is `POST [channel]/services` ([story 6](06-create-shared-channel.md)).

## API sketch

Names come from the stories; the shape is what we are agreeing on.

### A service

A service is a package: its `service.toml`, its schema, set up by its own `init`, and the handlers that read and write it.

```toml
# services/chat/service.toml
name = "chat"      # its canonical name; the name the app registers it under overrides it
timeout = 200      # milliseconds every call to it must settle within; 200 when left out

[[sql]]
binding = "db"     # the storage it asks for, handed to its handlers as env.db; at most one

# [effects.fetch]
# replicate = "never"   # an effect it makes: "never", the default, makes it on an op's first run
#                       # only and replays its recorded output; "always" makes it on every run
```

```js
// services/chat/index.js
// The app names it when it registers it: carbon.services.add('chat', chat)
import {notFound, forbidden} from '@tlsft/service-error'
import {applyMigrations, readMigrations} from 'carbondb/migrations'

// Its manifest, which CarbonDB reads when the app registers it.
export const manifest = new URL('./service.toml', import.meta.url)

// Its migrations are its own: D1-style files in migrations/, applied by init on every open.
const migrations = readMigrations(new URL('./migrations/', import.meta.url))

export function init(db) {
  applyMigrations(db, migrations)
}

export const calls = {
  // Posting only adds a message, so it never forces a rewind.
  'POST [chat]/messages': {
    tags: ['append'],
    handler: ({input, op, env: {db}}) => {
      const id = op.id()
      db.run('INSERT INTO message (id, author, text, sent_at) VALUES (?, ?, ?, ?)',
        [id, op.author, input.text, op.time])
      return {id}
    }
  },

  // Only the author may edit their message.
  'PUT [chat]/messages/:id': {
    handler: ({path, input, op, env: {db}}) => {
      const message = db.get('SELECT author FROM message WHERE id = ? AND deleted_at IS NULL', [path.id])
      if (!message) throw notFound('No such message')
      if (message.author !== op.author) throw forbidden('Only the author may edit a message')
      db.run('UPDATE message SET text = ?, edited_at = ? WHERE id = ?', [input.text, op.time, path.id])
      return {id: path.id}
    }
  },

  'DELETE [chat]/messages/:id': {
    handler: ({path, op, env: {db}}) => {
      const message = db.get('SELECT author FROM message WHERE id = ? AND deleted_at IS NULL', [path.id])
      if (!message) throw notFound('No such message')
      if (message.author !== op.author) throw forbidden('Only the author may delete a message')
      db.run('UPDATE message SET deleted_at = ? WHERE id = ?', [op.time, path.id])
      return {deleted: true}
    }
  },

  // A read: answered from the local replica, never an op.
  'GET [chat]/messages': {
    handler: ({env: {db}}) => db.all('SELECT id, author, text, sent_at, edited_at FROM message WHERE deleted_at IS NULL ORDER BY sent_at')
  }
}
```

```sql
-- services/chat/migrations/0001_create_message.sql
CREATE TABLE message (
  id TEXT PRIMARY KEY,
  author TEXT NOT NULL,
  text TEXT NOT NULL,
  sent_at INTEGER NOT NULL,
  edited_at INTEGER,
  deleted_at INTEGER
) WITHOUT ROWID;
```

### An app's main()

The app code is the same single-node or replicated; only the TOML file differs. `[sync] mode = "none"` runs single-node, with no p2panda and no network, and calls that need peers, such as `POST [channel]/members`, reject with `503 UNREACHABLE`.

```js
import {configure} from 'carbondb'
import * as chat from './services/chat/index.js'

// The carbon handle is the system on this device.
const carbon = await configure('./carbondb.toml')
carbon.services.add('chat', chat)

// An identity, its first device, its own channel and its personal channel, with keys generated here.
await carbon.call('POST [@carbon]/identity', {slug: 'alice', device: 'laptop'})
const personal = carbon.ref('identity:alice').ref('personal:alice')

// A channel is one chat room, nested privately under the personal channel, naming [chat].
await personal.call('POST [channel]/channel', {name: 'Room'})
const room = personal.ref('channel:room')
await room.call('POST [channel]/services', {name: 'chat'})

// Any medium can carry the code; Sally joins with it on her device.
const {code} = await room.call('POST [channel]/members', {contact: sallyContact, access: 'write'})

// A call becomes an op: applied here first, then replicated to every member.
await room.call('POST [chat]/messages', {text: 'hello'})

// Reads run against the local replica.
await room.call('GET [chat]/messages')

// State changes, for a UI.
const stop = room.watch('GET [chat]/messages', render)
```

### HTTP is the app's adapter

CarbonDB binds no protocol (see [implementation.md](implementation.md#remote-access-stays-outside-carbondb)). An app that wants HTTP picks the channel and forwards to `call`; a service error carries its status, so the response is a few lines:

```js
const respond = (room) => async (request) => {
  const result = await room.call(keyOf(request), await request.json()).catch(error => error)
  return Response.json(result, {status: result.status ?? 200})
}
```

What the runtime registers with the core for a service, and the only shape the core sees. Apps never call this:

```js
// The core reads and validates the service.toml at manifestPath, versions the
// package by hashing its files, and refuses a service without one.
native.register(name, manifestPath,
  // init(db): runs on every open, inside a transaction the core holds.
  initOf(chat),
  // entry(context): every call routed to the service. The context carries the
  // request {method, resource, input, op, binding} and is settled once, with
  // what the handler returns or throws; past the service's timeout the call
  // fails with a 504 and a late answer is dropped. The context also makes the
  // handler's effects.
  entryOf(chat.calls))
```

## Implementation

See [implementation.md](implementation.md), [docs/replication.md](../../docs/replication.md), and [p2panda-sync.md](../p2panda-sync.md) for syncing services over p2panda.
