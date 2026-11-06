# Design goals: CarbonDB as a runtime for services

CarbonDB lets a service written in the shape of an ordinary backend service (named calls, SQL tables, handlers) run on every member's device, with the replication protocol taking the place of HTTP. This note records who is responsible for what, how an op is applied, and a first sketch of the API. [implementation.md](implementation.md) holds the full agreed design, [services.md](services.md) the builtin calls, [packages.md](packages.md) how services are packaged and versioned, and [oplog-examples.md](oplog-examples.md) worked examples.

## The promise to a service author

- **Ops are service calls.** A write is a call such as `POST [chat]/messages {text}`; CarbonDB carries it to every member of the space.
- **You define your calls.** A service declares its calls and MAY tag them with metadata to assist in merging state (for example, that a POST only appends). Ordering needs are derived from the HTTP method first: GET is never an op, PUT and DELETE are idempotent and scoped to the resource their path names, POST and PATCH promise nothing unless tagged.
- **CarbonDB orders, merges and rewinds.** Every replica applies the same ops in the same order, re-running the service's own handler, so business rules give the same answer everywhere. When a late op arrives, CarbonDB rewinds only as far as it must.
- **You can react to state changes.** `watch` re-runs a GET whenever a committed op writes a table it read, for interfaces. A trigger forwards a call's input and output to other services, so one service can follow another's calls.
- **Replication is the transport.** Every mutation goes through replication.  Handlers are called by the replication protocol rather than by HTTP. The author's own device applies the call first, through the same path every other replica uses.

## Domain Terms

- **Identity**: a person, logically. A signed log of the device keys added to it and revoked from it, each entry signed by an existing device (Keybase's model). The log lives in an identity space, which runs only [space], [identity] and [device], and whose only member is the identity's own group of devices. It holds only what may be published: private by default, exportable, and served to subscribers if configured. Permissions and custody attach to the identity. In p2panda it is a group of its devices, so giving an identity access gives all its devices access. Its id is a canonical hash of its `POST [identity]/origin` op, signed by the first device, so it stays the same as keys rotate; its slug is a local pet name, unique on this machine.
- **Device**: a physical machine enrolled in one identity. It generates its own keys, which never leave it; it signs, and authorizes subkeys. One machine holding several identities has separate device keys for each, so they cannot be linked. Its metadata (name, type, platform) is self-declared; no fingerprinting. A paper key is an offline device.
- **Personal space**: the private root of what an identity owns, created with it, like `/home` beside `/`. It holds refs to the spaces the identity creates or joins, its devices' names and tags, and personal data. Others, such as a caretaker, may join it without being conflated with the identity.
- **Space**: a unit of replication with its own log and SQLite database. Membership decides access; when a device syncs a space is a setting. It runs an immutable set of services fixed when it is created. A personal space starts with one member. A space is not a tenant: a tenant holds many spaces.
- **Service**: a module of tables and calls keyed by service key (`POST [chat]/messages`). Its schema version is the number of its migrations.
- **Registered services**: the services an app registers in `main()` on every start with `carbon.services.add(name, impl)`. A device applies ops only for services it has registered.
- **system**: the root space, this device's local database and op log, holding what must be readable before any identity opens, such as the identities on this device. It is never replicated. Every other space is reached from it through refs, and `carbon` is its handle. Each space has one handle.
- **Op**: a service call carried in a space's log, applied by every member's device in one shared order. An op on a local service never replicates; it is kept in the op log of the space it ran in, marked with the device that ran it, or in the system log when made on the system handle.
- **Local service**: a service marked `[@name]`, running on this device in the context of whichever space it is called from. It can call replicated services, and they can never call it. Anyone can run one against a space without installing it there, such as a personal automation on a phone; the space still applies its rules to every op it writes.
- **Origin op**: a space's first op, `POST [space]/origin`, carrying what the space is: its kind, name, services and manage list. Those services fix what is in scope for the space.
- **Ref**: a pointer from a log to a space, written with `POST [space]/ref`. It grants nothing; membership grants access. A space has many refs, and a device finds spaces by walking them from its identity space, through its personal space.
- **Manage**: CarbonDB's own permission, listed on the origin op. CarbonDB maps it onto p2panda's Manage, which p2panda grants only to individual devices. With an empty list, no one may delete the space or add members, and it dissolves when all its members leave.
- **[@carbon]**: the local service for this device's own CarbonDB, available in every space. It does what must happen once, on one device: generating keys, creating and joining spaces, and changing members and manage lists in p2panda. It handles its own namespace; no other service may.
- **[space]**: the core service running in every replicated space, holding its origin, refs, members and manage list.
- **[identity]**: CarbonDB's own service for an identity's origin, running only in identity spaces. An app may run its own service of the same name, such as one holding roles, in other spaces.
- **[device]**: CarbonDB's own service for an identity's devices. In the identity space, `POST [device]/origin` records a device's public key, which authorizes that key to act for the identity. In the personal space, `POST [device]/ref` records each device's name and tags, the way `[space]/ref` points to a space from another one.

## Responsibilities

### CarbonDB core (Rust, loaded into Node through napi-rs)

- **Networking and membership, from p2panda:** the node, spaces and their members.
- **Identity:** the device constellation (enroll a device by QR code, revoke one, recover with a paper key) and the blind relay, per the README.
- **The op envelope:** device (signs and syncs), author (the person, for access control and audit), hybrid logical clock time, schema version, and the call itself as a body the core does not interpret beyond its service key.
- **The shared order:** ops sorted by (time, device, op hash); duplicates dropped.
- **SQLite:** one database of application tables per space, and the calls a handler uses to read and write it.
- **Undo:** each op's writes recorded as a SQLite session changeset; rewinding applies the inverse changesets of the ops a late op must re-run, newest first.
- **The apply-and-rewind loop:** the core owns the order and the changesets, so it decides when a handler runs or re-runs, and calls the service runtime to run it.
- **Notices:** one per commit to the service runtime, with the op, its outcome and the tables it touched. `watch` is built on them, and apps get no separate event API.
- **Builtin services:** `[@carbon]`, `[space]`, `[identity]` and `[device]`, implemented in Rust.

### Repeatability of State Mutations

Every state change would fall into one of three classes:

┌────────────────┬──────────────────────────┬───────────────────────┐
│     Class      │         Example          │         Rule          │
├────────────────┼──────────────────────────┼───────────────────────┤
│ Undoable       │ SQLite writes            │ Undone and re-run by  │
│                │                          │ rewind                │
├────────────────┼──────────────────────────┼───────────────────────┤
│ Repeatable,    │ Content-addressed blob   │ Safe in any order, no │
│ grow-only      │ puts                     │  undo needed; the op  │
│                │                          │ carries the hash      │
├────────────────┼──────────────────────────┼───────────────────────┤
│                │ p2panda removal,         │ Driven from committed │
│ Irreversible   │ deleting a blob, sending │  state after the call │
│                │  email                   │                       │
└────────────────┴──────────────────────────┴───────────────────────┘

Deleting a blob is the CALM case: it retracts a fact, so it should be garbage collection driven by settled state, not something a handler does. The email row is the weak spot, because a rewind can't recall an email once sent. Creating a space is irreversible too, so it is a local op that runs once, on one device.

TODO: expand this as a set of rules that services need to follow in order to ensure eventual consistency.

### Service runtime (JavaScript)

- **Routing:** a service key to its handler. Validating input is the service's own business.
- **The handler context:** the call's input and path parameters, the author, and time, ids and randomness taken from the op so every replica computes the same values.
- **Migrations:** the service's table definitions.
- **Access:** whether this author may make this call.
- **Later:** richer service runtimes, such as one that derives schema defaults from the op and builds queries from data, run on the same core.

### What the CarbonDB core never knows

The schemas, routes, validation, roles and merge strategy of app services, beyond the shared order. A service is ops in and SQLite writes out; what happens in between is the service's business.

## Applying an op

1. The core picks the next op in the shared order and opens a transaction and a session on the space's database.
2. It calls the service runtime's `apply` with the op and a database handle, and awaits the Promise it returns. Handlers may be async; the core waits for the Promise to settle before it continues.
3. The handler reads and writes through the database handle. Those calls are synchronous and run inside the open transaction.
4. When the Promise resolves, the core commits and keeps the session's changeset with the op. When it rejects or returns an error, the core rolls back and records the op's result as refused, with its error.
5. One op at a time per space, since the transaction stays open across the handler's awaits.

## Undo, write sets and read sets

- **Session extension (the target for undo).** SQLite records the rows an op wrote, with old and new values. That changeset is both the undo record (rusqlite has `Changeset::invert`) and the op's write set. rusqlite is pinned to 0.39.0, the newest version that links beside p2panda's sqlx. That has compiled and run in a scratch crate, but not yet beside a live p2panda node.
- **Authorizer (read sets in the core).** SQLite reports every table and column a statement reads as it is prepared. That gives each op's read set at table and column level, without knowing any schema. It cannot see which rows a WHERE clause matched.
- **Why read sets.** When a late op arrives, an op after it must re-run if it read anything the late op wrote. Write sets alone cannot tell that.
- **Finer read sets, later.** A runtime that builds queries from data knows predicates such as `year = 2025`, so it can report reads per row. A late op that writes a 2024 account then forces no rewind of an op that read 2025's accounts; with table-level read sets it would.

## Stories

1. [A person can initialize their identity on a new device](01-initialize-identity.md)
2. [A person can add another device to their identity](02-add-device.md)
3. [A person can choose among the identities on this device](03-list-identities.md)
4. [An app developer can register the services the app runs](04-register-services.md)
5. [A person can call a service in their personal space](05-call-personal-space.md)
6. [A person can create a shared space naming its services](06-create-shared-space.md)
7. [A person can add members to a shared space](07-add-members.md)
8. [A person can call a service in a shared space](08-call-shared-space.md)
9. [A person can see changes in a space as they arrive](09-watch-space.md)

### Backlog

- [A person can revoke a lost device](backlog/revoke-device.md)
- [A manager can remove a member, and a member can leave or delete a space](backlog/remove-member.md)
- A person grants remote access with roles without membership
- [A person learns a call was rejected after merge](backlog/rejected-after-merge.md)
- [Old ops are refused or re-run safely across versions and time](backlog/old-ops.md)
- CarbonDB identity carries app-defined vouchers
- [A person can host services on heavy devices and reach them from light ones](backlog/heavy-and-light-devices.md)
- [CarbonDB gets a security and privacy review after the prototype](backlog/security-review.md)
- [Replicas are proven to hold the same ops when a member is removed](backlog/verify-replica-op-sets.md)
- [A manager can move a space to a different parent](backlog/reassign-space-parent.md)
- [A company can allow or prevent adding a member to its team](backlog/parent-rules-on-child-members.md)
- [Anyone can verify an identity's devices and public proofs](backlog/public-identity-proofs.md)
- [A manager can add a service to an existing space](backlog/add-services-to-space.md)
- [A manager can move a space to a new version of a service](backlog/upgrade-package-version.md)
- [A service can store files in a space](backlog/file-bindings.md)
- [An app developer can install @tlsft/carbondb on any desktop platform](backlog/native-packaging-per-platform.md)
- [An iOS user runs the app's shipped services at native speed](backlog/ios-ahead-of-time-packages.md)
- [An app developer's npm service runs at the version npm installed](backlog/npm-package-integrity.md)
- [An app developer can run a Worker's D1 queries on CarbonDB unchanged](backlog/d1-compatibility.md)
- [A person's interrupted call finishes when the app restarts after a crash](backlog/resume-after-crash.md)
- [A person gets a paper key when they create an identity](backlog/paper-key.md)

## API sketch

Names come from the stories; the shape is what we are agreeing on.

### A service

```js
// services/chat.js: a service is its tables (in SQLite)
// and the handlers that read/write from those tables
// The app names it when it registers it: carbon.services.add('chat', chat)
import {notFound, forbidden} from '@tlsft/service-error'

export const migrations = [
  `CREATE TABLE message (
     id TEXT PRIMARY KEY,
     author TEXT NOT NULL,
     text TEXT NOT NULL,
     sent_at INTEGER NOT NULL,
     edited_at INTEGER,
     deleted_at INTEGER
   )`
]

export const calls = {
  // Posting only adds a message, so it never forces a rewind.
  'POST [chat]/messages': {
    tags: ['append'],
    handler: ({input, op, db}) => {
      const id = op.id()
      db.run('INSERT INTO message (id, author, text, sent_at) VALUES (?, ?, ?, ?)',
        [id, op.author, input.text, op.time])
      return {id}
    }
  },

  // Only the author may edit their message.
  'PUT [chat]/messages/:id': {
    handler: ({path, input, op, db}) => {
      const message = db.get('SELECT author FROM message WHERE id = ? AND deleted_at IS NULL', [path.id])
      if (!message) throw notFound('No such message')
      if (message.author !== op.author) throw forbidden('Only the author may edit a message')
      db.run('UPDATE message SET text = ?, edited_at = ? WHERE id = ?', [input.text, op.time, path.id])
      return {id: path.id}
    }
  },

  'DELETE [chat]/messages/:id': {
    handler: ({path, op, db}) => {
      const message = db.get('SELECT author FROM message WHERE id = ? AND deleted_at IS NULL', [path.id])
      if (!message) throw notFound('No such message')
      if (message.author !== op.author) throw forbidden('Only the author may delete a message')
      db.run('UPDATE message SET deleted_at = ? WHERE id = ?', [op.time, path.id])
      return {deleted: true}
    }
  },

  // A read: answered from the local replica, never an op.
  'GET [chat]/messages': {
    handler: ({db}) => db.all('SELECT id, author, text, sent_at, edited_at FROM message WHERE deleted_at IS NULL ORDER BY sent_at')
  }
}
```

### An app's main()

The app code is the same single-node or replicated; only the TOML file differs. `[sync] mode = "none"` runs single-node, with no p2panda and no network, and calls that need peers, such as `POST [@carbon]/members`, reject.

```js
import {configure} from '@tlsft/carbondb'
import * as chat from './services/chat.js'

// The carbon handle is the system space on this device.
const carbon = await configure('./carbondb.toml')
carbon.services.add('chat', chat)

// An identity, its first device and its personal space, with keys generated here.
const {personal} = await carbon.createIdentity({slug: 'alice', device: 'laptop', services: []})

// A space is one chat room, nested privately under the personal space.
const room = await personal.createSpace({name: 'Room', services: ['chat']})

// Any medium can carry the code; Sally joins with it on her device.
const {code} = await room.call('POST [@carbon]/members', {contact: sallyContact})

// A call becomes an op: applied here first, then replicated to every member.
await room.call('POST [chat]/messages', {text: 'hello'})

// Reads run against the local replica.
await room.call('GET [chat]/messages')

// State changes, for a UI.
const stop = room.watch('GET [chat]/messages', render)
```

### HTTP is the app's adapter

CarbonDB binds no protocol (see [implementation.md](implementation.md#remote-access-stays-outside-carbondb)). An app that wants HTTP picks the space and forwards to `call`; a service error carries its status, so the response is a few lines:

```js
const respond = (room) => async (request) => {
  const result = await room.call(keyOf(request), await request.json()).catch(error => error)
  return Response.json(result, {status: result.status ?? 200})
}
```

What the runtime registers with the core for a space, and the only shape the core sees. Apps never call this:

```js
core.apply(space, {
  migrations: chat.migrations,
  tags: {'POST [chat]/messages': ['append']},
  // op: {key, path, input, author, device, time, id(), random()}; db: {run, get, all}
  apply: async (op, db) => routeTo(chat.calls, op.key).handler({path: op.path, input: op.input, op, db})
})
```

## Implementation

See [implementation.md](implementation.md).
