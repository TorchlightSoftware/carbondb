# Design goals: CarbonDB as a runtime for services

CarbonDB lets a service written in the shape of an ordinary backend service (named calls, SQL tables, handlers) run on every member's device, with the replication protocol taking the place of HTTP. This note records who is responsible for what, how an op is applied, and a first sketch of the API.

## The promise to a service author

- **Ops are service calls.** A write is a call such as `POST [chat]/messages {text}`; CarbonDB carries it to every member of the space.
- **You define your calls.** A service declares its calls and MAY tag them with metadata to assist in merging state (for example, that a POST only appends). Ordering needs are derived from the HTTP method first: GET is never an op, PUT and DELETE are idempotent and scoped to the resource their path names, POST and PATCH promise nothing unless tagged.
- **CarbonDB orders, merges and rewinds.** Every replica applies the same ops in the same order, re-running the service's own handler, so business rules give the same answer everywhere. When a late op arrives, CarbonDB rewinds only as far as it must.
- **You can listen for state changes.** CarbonDB reports each op applied, rejected or rewound, and which tables it touched.
- **Replication is the transport.** Every mutation goes through replication.  Handlers are called by the replication protocol rather than by HTTP. The author's own device applies the call first, through the same path every other replica uses.

## Domain Terms

- **Identity**: a person, logically. A signed log of the devices added to it and revoked from it, each entry signed by an existing device (Keybase's model). Permissions and custody attach to the identity. Its id is a canonical hash of its public key; its slug is a local pet name, unique on this machine.
- **Device**: a physical machine enrolled in one identity. It generates its own keys, which never leave it; it signs, and authorizes subkeys. One machine holding several identities has separate device keys for each, so they cannot be linked. Its metadata (name, type, platform) is self-declared; no fingerprinting. A paper key is an offline device.
- **Space**: a unit of replication with its own log and SQLite database. Its members host it; it runs an immutable set of services fixed when it is created. A personal space starts with one member. A space is not a tenant: a tenant holds many spaces.
- **Service**: a module of tables and calls keyed by service key (`POST [chat]/messages`). Its schema version is the number of its migrations.
- **Catalog**: the services this app can run, recorded once in SQLite by module path.
- **Op**: a service call carried in a space's log, applied by every member's device in one shared order.
- **[carbon]**: the implicit service for shared actions (creating a space, members, listeners). It handles its own namespace; no other service may.

## Responsibilities

### CarbonDB core (Rust, loaded into Node through napi-rs)

- **Networking and membership, from p2panda:** the node, spaces and their members.
- **Identity:** the device constellation (enroll a device by QR code, revoke one, recover with a paper key) and the blind relay, per the README.
- **The op envelope:** device (signs and syncs), author (the person, for access control and audit), hybrid logical clock time, schema version, and the call itself as a body the core does not interpret beyond its service key.
- **The shared order:** ops sorted by (time, device, op hash); duplicates dropped.
- **SQLite:** one database of application tables per space, and the calls a handler uses to read and write it.
- **Undo:** each op's writes recorded as a SQLite session changeset; rewinding applies the inverse changesets of the ops after a late op, newest first.
- **The apply-and-rewind loop:** the core owns the order and the changesets, so it decides when a handler runs or re-runs, and calls the service runtime to run it.
- **Events:** applied, rejected and rewound, per op, with the tables it touched.

### Service runtime (JavaScript)

- **Routing:** a service key to its handler, and input validation.
- **The handler context:** the call's input and path parameters, the author, and time, ids and randomness taken from the op so every replica computes the same values.
- **Migrations:** the service's table definitions.
- **Access:** whether this author may make this call.
- **Later, for Otto:** candle-db's replicated mode (schema defaults derived from the op, finer read tracking in QueryRunner) is a richer service runtime on the same core.

### What the CarbonDB core never knows

Schemas, routes, validation, roles and merge strategy beyond the shared order. A service is ops in and SQLite writes out; what happens in between is the service's business.

## Applying an op

1. The core picks the next op in the shared order and opens a transaction and a session on the space's database.
2. It calls the service runtime's `apply` with the op and a database handle, and awaits the Promise it returns. Handlers may be async; the core waits for the Promise to settle before it continues.
3. The handler reads and writes through the database handle. Those calls are synchronous and run inside the open transaction.
4. When the Promise resolves, the core commits and keeps the session's changeset with the op. When it rejects or returns an error, the core rolls back and records the op as rejected.
5. One op at a time per space, since the transaction stays open across the handler's awaits.

## Undo, write sets and read sets

- **Session extension (the target for undo).** SQLite records the rows an op wrote, with old and new values. That changeset is both the undo record (rusqlite 0.41 has `Changeset::invert`) and the op's write set. To be confirmed: rusqlite and p2panda's sqlx linking one SQLite library in the same binary.
- **Authorizer (read sets in the core).** SQLite reports every table and column a statement reads as it is prepared. That gives each op's read set at table and column level, without knowing any schema. It cannot see which rows a WHERE clause matched.
- **Why read sets.** When a late op arrives, an op after it must re-run if it read anything the late op wrote. Write sets alone cannot tell that.
- **QueryRunner (finer read sets, later).** A runtime that builds queries from data, as candle-db does, knows predicates such as `year = 2025`, so it can report reads per row. A late op that writes a 2024 account then forces no rewind of an op that read 2025's accounts; with table-level read sets it would.

## Stories

1. [A person can initialize their identity on a new device](01-initialize-identity.md)
2. [A person can add another device to their identity](02-add-device.md)
3. [A person can choose among the identities on this device](03-list-identities.md)
4. [An app developer can catalog the services the app runs](04-catalog-services.md)
5. [A person can call a service in their personal space](05-call-personal-space.md)
6. [A person can create a shared space naming its services](06-create-shared-space.md)
7. [A person can add members to a shared space](07-add-members.md)
8. [A person can call a service in a shared space](08-call-shared-space.md)
9. [A person can see changes in a space as they arrive](09-watch-space.md)

## API sketch

Names are provisional and are being replaced story by story with names from the service domain; the shape is what we are agreeing on.

### A service

```js
// services/chat.js: a service is its tables (in SQLite)
// and the handlers that read/write from those tables
export const name = 'chat'

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
      if (!message) return {error: 404}
      if (message.author !== op.author) return {error: 403}
      db.run('UPDATE message SET text = ?, edited_at = ? WHERE id = ?', [input.text, op.time, path.id])
      return {id: path.id}
    }
  },

  'DELETE [chat]/messages/:id': {
    handler: ({path, op, db}) => {
      const message = db.get('SELECT author FROM message WHERE id = ? AND deleted_at IS NULL', [path.id])
      if (!message) return {error: 404}
      if (message.author !== op.author) return {error: 403}
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

### Single-node: plain Node and SQLite

The same service on one machine, with no CarbonDB: a call is applied directly, in a transaction, with the op context made locally.

```js
import {openLocal, call} from './runtime/local.js'
import * as chat from './services/chat.js'

const room = openLocal(chat, {file: 'room.db', author: 'brandon'})
await call(room, 'POST [chat]/messages', {text: 'hello'})
await call(room, 'GET [chat]/messages')
```

### CarbonDB: the same service, replicated

```js
import {openNode} from '@tlsft/carbondb'
import {serve, call} from './runtime/carbondb.js'
import * as chat from './services/chat.js'

// This device: its identity, its keys, its p2panda node.
const node = await openNode({dir: './data'})

// A space is one chat room; joining it makes this device a replica.
const room = await node.createSpace()                 // or node.joinSpace(ticket)
const ticket = await node.invite(room, {contact, access: 'write'})

// The runtime hands the core one apply function for the space; the core calls it for every op, in order.
serve(node, room, chat)

// A call becomes an op: applied here first, then replicated to every member.
await call(node, room, 'POST [chat]/messages', {text: 'hello'})

// Reads run against the local replica.
await call(node, room, 'GET [chat]/messages')

// State changes, for a UI.
node.subscribe(room, ({type, op, tables}) => { /* applied | rejected | rewound */ })
```

What `serve` registers with the core, and the only shape the core sees:

```js
node.apply(room, {
  migrations: chat.migrations,
  tags: {'POST [chat]/messages': ['append']},
  // op: {key, path, input, author, device, time, id(), random()}; db: {run, get, all}
  apply: async (op, db) => routeTo(chat.calls, op.key).handler({path: op.path, input: op.input, op, db})
})
```

## Implementation (next)

1. Declare the [chat] service in plain Node.
2. Run it in single-node mode on SQLite, with tests.
3. Implement CarbonDB first draft against the binding shapes above.
4. Build out the CarbonDB core: napi-rs bindings, the apply loop, sessions, p2panda spaces.
