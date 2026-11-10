# CarbonDB first draft: implementation notes

Status: agreed — single-node built (build steps 1 to 3), with the op log and results kept apart, per-service namespacing and init(), and a local op carrying its sub-calls on its log row; replicated mode not started, so nothing replays sub-calls yet

This is everything we agreed that the code has to honour, gathered in one place for whoever builds the first draft. The nine numbered stories say what a person can do, [services.md](services.md) lists the builtin calls, and [oplog-examples.md](oplog-examples.md) shows them in use. This file covers what happens underneath. Where older notes disagree with it, this file wins.

The spec is the app interface: what an app calls and what CarbonDB guarantees in return. Everything beneath it may change as the build-out needs, as long as that interface and its guarantees hold. Sections after [Remote access stays outside CarbonDB](#remote-access-stays-outside-carbondb) describe replicated mode unless they say otherwise.

## Build order

1. **Single-node CarbonDB:** the Rust core behind napi-rs, with no p2panda, delivering the whole app interface below. Ops already go through each space's OpWriter, op log, pending ops and results, the same path synced ops will take, and each op already records its session changeset.
2. **[chat] on it:** `main()` loads CarbonDB, registers [chat] with `carbon.services.add('chat', chat)` and gets a handle. [chat] stays small, resembling an ordinary backend service with minimal dependencies, with plain SQL and no build step.
3. **A small JS HTTP adapter** in front of [chat]. The tests talk to it over HTTP.
4. **Replicated mode:** rewind and p2panda spaces, switched on by config with no change to the app.

The hello world is create a space, post, edit, delete, list and watch, on single-node CarbonDB. Adding a member and joining (story 7) arrive with replicated mode. Richer service runtimes come later on the same app interface.

## The app interface

- **Setup:** an app's `main()` loads CarbonDB with its TOML config, registers each service with `carbon.services.add(name, impl)`, where `impl` is the service's exported object, and gets the `carbon` handle. Whether it runs single-node or replicated is a config setting, so switching is a config change, not a refactor of the app.
- **Handles:** one per space, reached through refs from `carbon` (see [Handles](#handles)).
- **`call(key, input)`:** runs the service's handler and resolves with its result once this device has applied it, or rejects with a service error.
- **`watch(key, fn)`:** re-runs a GET and hands `fn` the result after each commit that touched what it read, and returns a function that stops it.
- **Service errors:** a rejection carries an HTTP-style status, a code and details, preserved across the core boundary (see [Service errors](#service-errors)).
- **Handlers** receive `path`, `input`, `db` and `call`, and `op` when the call writes. `op` carries the key, path, input, the verified author and device as plain data, the time, `id()` and `random()`. `call(key, input)` makes a call inside this one (see [Applying an op](#applying-an-op)).

The host function handlers use for SQL is internal and is not part of the spec.

### Single-node mode

```toml
[sync]
mode = "none"   # single-node: no p2panda node is started
```

- The same app interface and builtin services, on the Rust core with no p2panda and no network.
- Identities, spaces and refs exist locally, so handles reach spaces the same way.
- The op log is kept from the first draft, so local calls and synced ops share one apply path from the start.
- Calls that need p2panda reject: `POST [@carbon]/members`, `POST [@carbon]/join`, `POST [@carbon]/sync/start`, and `POST [device]/origin` for a second device.
- `GET [@carbon]/status` answers `sync: 'off'`, `peers: []`, and full access for the local identity, since nothing else answers who has access.
- A single-node instance cannot switch to replicated later. Its data does not carry over.

### Service errors

- A service error is an Error with a numeric HTTP `status`, a string `code`, a message and details. The core guarantees that one thrown in a handler reaches the caller of `call` with status, code and details intact, so an adapter maps it to a response in a few lines.
- MicroQL's errors (`src/common/error-catalog.js` in the microql repo) already have that shape, as do those of other service runtimes we use, though their codes differ.
- The shared shape lives in `@tlsft/service-error`, beside `@tlsft/service-key`, and services throw its errors, such as `notFound` and `forbidden`. Reconciling the existing codes can follow.

## Remote access stays outside CarbonDB

CarbonDB does not bind services to HTTP, gRPC or any other protocol. An app that wants HTTP writes an adapter that turns a request into `space.call(key, input)`, and a service error into a response: a request-to-response function, with something else doing the listening.

Why the boundary sits here:

1. **Serving a remote caller means deciding who that caller is.** That is authentication and RBAC, which CarbonDB leaves to optional services, such as an app's role service. CarbonDB knows only its members' signed identities.
2. **Spaces have no network address.** Spaces nest and act as virtual networks, with no tie to a transport, a network boundary or an IP address. Binding one to an external address carries assumptions CarbonDB should not make, so the adapter decides which space a request reaches.
3. **Not every service key is HTTP.** The HTTP method in a service key is optional, and services may be local-only or served over gRPC.

The long-term case still fits: peers who should call a space's services remotely without holding its data get finer read control than replication allows. Approving their identities and serving them is app or runtime work, not CarbonDB's.

## Prove these first

None of this has run on real p2panda nodes yet. Each item is a design risk for replicated mode, so test it early:

- The invitation flow in story 7: subscribe, key bundle arrives, a manage device adds the member, and the new member reads earlier messages.
- rusqlite 0.39.0 beside a live p2panda node in one binary. So far it has only compiled and inverted a changeset beside sqlx-sqlite 0.9.0 in a scratch crate.
- An identity group as a space member, including a device added after the space was created.
- Ops at or below a removal cutoff reaching every replica (see [the backlog test](backlog/verify-replica-op-sets.md)).
- Rewind cost under realistic load: ops per space, how deep a late op lands, changeset storage and apply latency. Performance claims are measured, not estimated.

## Stack

- **Packaging:** one native binary per platform. JS apps drive it as a library, and the Node binding is a napi-rs native module.
- **Handlers:** service handlers run in the host Node process and call the core for storage and sync. Route definitions stay in JS. Validating a call's input is the service's own business: a handler that validates throws `validationError`, and CarbonDB never sees its schema.
- **SQLite:** real SQLite in the Rust core. Internally, Rust exposes one `execute(sql, params, method)` host function to handlers, where `method` is Drizzle sqlite-proxy's run, all, get or values.
- **rusqlite:** pinned to 0.39.0 with the `session` and `bundled` features. Version 0.41.1 cannot link beside sqlx 0.9, because it needs libsqlite3-sys 0.39 and sqlx-sqlite 0.9.0 needs below 0.38.
- **p2panda:** provides transport, signed per-author logs and spaces, so sqlite-sync and cr-sqlite are not needed. Only capture and merge are ours.

## Files on disk

- The TOML file names the system database, which holds identities and the system log. `configure` reads it once, and no other call takes a directory.
- Each space file sits beside the system database, named by space id.
- Each identity has its own p2panda store file beside it, because a p2panda node takes one set of credentials and one machine may hold several unlinkable identities. That means one p2panda node per identity on a device.
- A space's op log and changesets live in its own file, so they commit atomically with application writes. The session records every table but the op log and results, using rusqlite's `table_filter`.
- The core's own tables, the op log and results in every space file and the identities in the system database, move forward by migrations, tracked in `carbon_migrations` and applied in one transaction when the file opens. The system database's file applies a space's migrations, then its own. Until 1.0 these migrations may be consolidated, and development databases deleted when they are.

## Spaces and services

- **system** is the root space, local and never replicated. It holds what must be readable before any identity opens. It is a plain local log, not a p2panda space, because a p2panda space needs a node and a node needs an identity. `carbon` is its handle.
- **Names:** a space is named by its kind and a lowercase kebab-case slug, such as `identity:alice`, `company:foo-corp` and `team:sales`. The root is `system`.
- **Kind** is a free label for apps and interfaces. CarbonDB gives meaning only to `identity` and `personal`, the kinds it creates itself.
- **Local services,** marked `[@name]`, run on this device in the context of the space they are called from. They are often built to facilitate system calls which need to be called once and only once.  They can call replicated services, and often do in order to set up replicated record keeping. Replicated services can never call local services, because this would cause them to duplicate their effects. This means that for now a replicated handler therefore cannot create a space - we may revisit this.
- **`[@carbon]`** is the core's local service, implemented in Rust because it reaches keys and p2panda. Its API calls must happen once, on one device.
- **`[space]`** runs in every replicated space: its origin, refs, members and manage list.
- **`[identity]`** runs in identity spaces: the identity's origin. An app may run its own service of the same name, such as one holding roles, in other spaces; the two never share a space.
- **`[device]`** follows `[space]`'s pattern: `POST [device]/origin` in the identity space authorizes a device's public key, and `POST [device]/ref` in the personal space records its name and tags. A second ref for the same key replaces the first.
- **Local ops:** an op on a local service is kept in the op log of the space it ran in, marked with the device that ran it, and replicated like any other once it has applied, so the log shows every action behind a space's state. Its sub-calls, the calls it made to replicated services in that space whose writes committed, are written onto its log row in the order made, each with its own stamp and input, in the same transaction that records its result. A refused sub-call changed nothing, so it is not carried; its outcome stays in the local op's result. Only local ops carry sub-calls: a replicated op replays by re-running itself, calls included. Every replay of a local op, on other devices and in a rewind on the device that ran it, runs its sub-calls in one transaction and never the local op itself, so a local op stays as atomic everywhere as it was where it ran. Calls on the system handle, which belong to no space, go in the system log, the system database's op log, with their results there. Either gives one place to finish interrupted work after a crash.

### Creating a space

1. `POST [@carbon]/space` runs in the context of the handle it is called on, so the new space is nested there.  This is facilitated by a `ref` record in the current space.
2. Validity of `refs` is checked before creating the space.  If the permissions of any space reject the `ref` then the space will not be created.  In essence this means that you cannot create child spaces unless you have "manage" permission on the current space.
3. The `[@carbon]` handler creates the space in p2panda. The space id is the hash of the local op, so a retry after a crash reaches the same space. `create_space` accepts any hash as an id.
4. The new space's first op is `POST [space]/origin`, carrying its kind, name, services and manage list. It repeats the create call's fields because the space's members may not be able to read the log it was created from.

Creating a space cannot be undone. If a rewind rejects a parent's ref, the space stays, unattached. The [security review](backlog/security-review.md) revisits this.

### Rules on `[space]` and origin ops

- Every POST to `[space]` requires "manage" on the space.
- An origin op can be written only once: `[space]` and `[identity]` once per space, `[device]` once per public key.
- The services named in a space's origin op fix what services run there, so a call to anything else is rejected.
- The set of services is fixed at creation for the first draft (see [the backlog story](backlog/add-services-to-space.md)).

### Hierarchy

- Spaces do not nest in p2panda. Adding a group as a member gives everyone in it access to the space, which is not what a parent and child mean, so p2panda nesting is used only for the devices of an identity.
- Hierarchy lives in refs and manage lists. A ref only points and grants nothing, and a space may have many refs.
- Parenthood is set at creation. Reassigning it is undefined (see [the backlog story](backlog/reassign-space-parent.md)).
- An employee identity reaches a team space by first looking in its company, because that access comes through the company. Only a direct relationship, such as a consultant, puts a team ref directly in a personal space.
- Reaching a space through refs needs the refs to exist and the identity to have access. What is possible in each space depends on that access.
- Adding someone to a child space does not add them to its parent.  But it may be prevented based on RBAC in the parent (see [the backlog story](backlog/parent-rules-on-child-members.md)).
- `cascade` on a ref takes `remove`, where losing access to the parent removes access to the child, and `delete`, where deleting the parent deletes the child, given manage rights there. Teams within a company use both.

## The op

- An op is a service call: service key plus fields, plus op hash, device (signs and syncs), author (the person, for access control and audit), hybrid logical clock time and version: the hash of the package that handles it.
- For now, a device that does not run the version an op names refuses that op (see [old ops](backlog/old-ops.md)).
- The author comes from the signed envelope, never from a field on the call.
- The op hash is computed over CarbonDB's signed envelope before the handler runs. A local op's sub-calls are not part of its call, so the hash does not cover them. It is separate from the p2panda operation hash, which does not exist yet at that point and covers ciphertext.
- Ops relate through the hybrid logical clock, not through lists of other ops. The one exception is the per-device removal cutoff.
- CarbonDB treats an op's input as opaque data. A handler error, including the service's own input validation, rejects the op.
- The op log, aligned with p2panda, is the space's audit log.
- GET is never an op. PUT and DELETE are idempotent and scoped to the path they name, and POST and PATCH promise nothing unless tagged. A POST may carry an optional tag such as `append`.

## Applying an op

- Every replica sorts ops by (hybrid logical clock, device, op hash), drops duplicates, and re-runs the service's handler in that order, so business rules give the same result everywhere.
- The core owns the order and the changesets, so it decides when a handler runs or re-runs. It routes every call by service name to the service's entry point, the same for Rust builtins, npm services and WASM packages, and the entry point settles it once.
- The op log records what this device received, local calls and synced ops alike. A space's OpWriter writes each op as it arrives, before it applies, as a durable queue would, so a crash loses nothing. While an op applies, its transaction holds the file, so an op arriving meanwhile is held by the OpWriter and written as soon as that turn ends, before the next op applies. Rows are never changed, except that a local op's sub-calls are written onto its row when it commits (see Local ops).
- Results record what this device did with each op: applied or refused with its service error, with its output and the calls it made. A device replaying another device's local op works out the same result locally from its sub-calls. Each runs under its own savepoint, so a sub-call refused on replay, as after a rewind, rolls back only its own writes. A result is keyed by its op hash and never leaves this device. A rewind replaces results; it never touches the log.
- The pending ops are the op log minus the results: the ops with no result yet. They are read from the space's file each turn, never kept in memory, and apply one at a time in the shared order, so ops left without a result by a crash simply apply next time the space opens. A local call is stamped when it arrives, since that is when it is written; an op that arrives later but sorts earlier is handled by rewind.
- Ops apply one at a time per space, and spaces apply side by side. A space is the finest unit: its services share one SQLite file, which has one writer, an op holds its transaction while its handler awaits, and one op's calls span services in the space.
- Every op takes the same path, local or synced: the OpWriter logs it, it applies in turn, and its result is recorded. Nothing is refused before it is logged, so every failure is reported, audited and kept in the history the same way.
- Pending ops apply only once every service the space names is set up on this device; until then the space is initializing, and its ops and their callers wait.
- A call made on this device to a service it does not run carries the version `service missing`. It is logged, and refused when its turn comes, before any handler sees it; that refusal keeps it from running again and from being published.
- The callers waiting on an op are kept beside it in memory, by op hash, and answered when its result is recorded. An op resumed from the log has none.
- Each space has a RunState: initializing, with the named services not set up here yet; idle; applying an op; or stopped. The app can read it and watch it.
- A call a handler makes runs inside the same op, under a savepoint: it sees the writes before it, and when it fails only its own writes roll back and the error reaches the handler that made it. A GET inside an op reads through the op's transaction. When a local service calls a replicated service in the same space, the call also takes a stamp of its own and is recorded as a sub-call on the local op's log row, so other devices can replay it without the local op (see Local ops). Replicated services never call local ones.

The steps for each op:

1. The core opens a transaction on the space's database, with a session recording its changes.
2. It hands the call to the service's entry point and waits for it to settle. Handlers may be async.
3. Database calls from the handler are synchronous and run inside the open transaction.
4. When the Promise resolves, the core records the op's result and commits, taking the session's changeset; replicated mode keeps it with the op. When it rejects or returns an error, the core rolls back and records the op's result as refused, with its error.
5. The core keeps every watch: a GET, the tables it read, and when it must run again after a commit that changed one of them, or after its space stops or resumes. A write from the app is answered once the watches its commit owes a run have run again: those that read a table it wrote, and those in flight when it landed, which are judged when they answer. A GET from the app answers at once. The runtime only registers watches and hands each answer to the app, and apps get no separate event API.

Timeouts and refusals:

- A call refused on the author's device is not published.
- A rejection on a later replica leaves the op in the log; its result records the refusal, on that device only, and `[@error]` reads it (see [rejected after merge](backlog/rejected-after-merge.md)).
- Every op stays in the log, so no data is lost. Each space gets a standard op history interface for finding a culprit op, such as a destructive offline edit, and reverting it. A revert is a new compensating op.
- A handler that overruns a timeout rolls back. Each service names its timeout in its `service.toml`, 200ms by default, and one deadline covers a call and the calls it makes. The core has no timers, so the host schedules the deadline, and the core settles the call with a `504` when it passes and drops its handler's later answer. A local call is then refused and never published. A synced op is not recorded as rejected, because a timeout differs between replicas, so the space pauses, reports the error and retries.

System failures:

- A system failure is SQLite failing in itself, not the op: an I/O error, a full disk, the file busy or locked by another process, corruption, or memory. The store classifies them by SQLite's error code on every statement, a handler's own included, and keeps the first it meets, so a handler that catches the error cannot hide it. No statement waits for a lock.
- Writing an op to the log or its result is never the op's own failure: when it fails the space stops too.
- A stopped space neither logs nor applies ops. The op that was applying rolls back and stays pending, every caller waiting on its ops gets a 503 carrying the failure, and so does every new call and every watch. It stays stopped until the problem is fixed by hand and the space is resumed, likely from a button in the app; then its pending ops apply from the first. A failure stops only the space it hit.

The runtime checks access while applying an op, against the space's state at that op's position in the shared order, so a late role change re-runs what read it. Replayed `[space]` handlers check manage from the log, not p2panda, because p2panda's current state differs between replicas.

### Reads

GETs and watch re-runs use a second read-only connection in WAL mode, so they see committed state and cannot write. They answer from this device's copy, even when it is behind other members. SQLite's authorizer reports what a GET reads, and a watch re-runs when a committed op's changeset changes a table it read. A batch of synced ops re-runs a watch once.

## Late ops and rewind

- Each op's writes are recorded as a SQLite session changeset, which is both its undo record and its write set. The authorizer gives its read set, by table and column.
- Rewind lives in its own module beside dispatch: it chooses the re-run set, inverts changesets newest first and re-runs. Inverting one changeset is a store primitive.
- Taking the write set from the changeset, in single-node mode too, was chosen over the authorizer's record of written tables:
  - It is exact. The authorizer counts a table as written when a statement naming it is prepared, even one that changes no rows; a changeset holds only net row changes, so a no-op update or an insert deleted again re-runs no watch.
  - It is one record. What a watch treats as written is exactly what rewind undoes and checks for overlap, so they cannot disagree.
  - It costs a session on every op in single-node mode too, recording old values and reading each changed table's columns once per op. This is to be measured, not estimated.
  - The core's own tables, the op log and results, leave the write set, since the session leaves them out. No watch reads them.
  - A savepoint rolled back inside the transaction leaves nothing in the changeset, as the [session spike](../spikes/session-rowid/README.md) shows, so a failed nested call neither re-runs watches nor reaches rewind.
- A late op whose reads and writes do not overlap later ops applies in place, with no rewind.
- Otherwise the re-run set is the transitive closure: every later op that read or wrote what the late op, or an op already in the set, wrote. Those changesets are inverted newest first, the late op applies, and the set re-runs in order. A local op in the set re-runs as its sub-calls, in one transaction, never as itself (see Local ops).
- The `append` tag is an author's promise that no handler depends on the row being absent. A late tagged op applies in place and re-runs nothing.
- The core treats tags as opaque flags. A tagged op whose changeset holds anything but inserts is rejected on every replica. A wrong promise about reads is not detected at runtime; the test harness catches it.
- A row's key comes from the op that creates it, never from the rows already in the table: a key SQLite assigns, a rowid or an AUTOINCREMENT id, depends on those rows, so after a rewind it can move under the row's author. After a service's `init`, the core checks every table it owns has a primary key that is not SQLite's rowid, and refuses the service with an error naming the table, what to change and why. Handlers take keys from `op.id()`, or an integer key from `DEFAULT (op_id())`, a SQL function every runtime registers that draws below 2^53. Sessions do not record schema changes, so schema changes only in `init`, outside any op.
- A later refinement can narrow read sets to rows through QueryRunner predicates.

## Publishing

- The op log is the outbox. A publisher drains this device's ops whose result applied, also after a crash; a call refused on its own device stays in the log unpublished, its result saying why. Duplicates drop by op hash.
- p2panda streams replay unacknowledged operations after a restart, so the core acknowledges after commit to cover a crash on the receiving side.
- A call resolves once this device has applied it, without waiting for other members.

## Repeatability

Every state change belongs to one class, as in the table in [README.md](README.md):

- **Undoable:** SQLite writes, undone and re-run by rewind. The calls a trigger forwards are in this class: they run in the applying transaction, so they rewind with the op that fired them.
- **Repeatable, grow-only:** such as content-addressed blob puts. No undo is needed.
- **Irreversible:** such as p2panda removal, creating a space or sending email. These are driven from committed state after the call.

## Rules for service authors

- All mutations go through the op log; nothing mutates a table directly.
- SQLite state is a pure fold of valid ops, and can be thrown away and rebuilt.
- Handlers take time, ids and randomness from the op context, never from `Date.now`, `randomUUID` or `Math.random`. The author picks ids and timestamps, and they travel in the op.
- Rules needing global knowledge, such as uniqueness or a non-negative balance, are modelled as visible conflicts or resolved deterministically.
- A row's key comes from the op that creates it: `op.id()`, or `DEFAULT (op_id())` in a WITHOUT ROWID table.
- All implementations of one service in a space must behave identically, or replicas drift silently. This is a documented rule, not a mechanism.
- Traditional models work with last-writer-wins from the shared order; better mergeable data structures come from schema design.

A test harness runs the same ops on two fresh databases in different delivery orders and compares state. There is no runtime detection.

## Identity and devices

- An identity is a p2panda group of its devices, so giving the identity access gives all its devices access.
- p2panda does not implement Manage for a group, so we implement our own Manage for Identities which maps it onto each Device within the Identity.
- The identity record is initiated by an "origin" log in an identity space.  The identity space runs the services `[space]`, `[identity]` and `[device]`. Its only member is the identity's group, and other identities cannot join.
- Other replicas tie a device to an identity through the p2panda group, without reading the identity log. This diverges from Keybase (see [Story: Anyone can verify an identity's devices and public proofs](backlog/public-identity-proofs.md)).
- The identity id is the hash of `POST [identity]/origin` (the first Op in the space), signed by the first device, so it stays the same as keys rotate.
- `POST [device]/origin` authorizes a public key to act for the identity, and the device making that call adds the new device to the p2panda group.
- The paper key is an identity log entry only, and stays out of the p2panda group until recovery is designed.
- Device additions commute, so this log needs no rewind for the current stories.
- Prefer Keybase semantics unless there is a well-justified reason to diverge. Having no server is one such divergence.

### No secrets in replicated ops

- Device private keys stay in system. Group secrets belong to p2panda, which keeps them per device and sends them only encrypted to each device. Callers reach either only through `[@carbon]`.
- There is no per-identity key. A device reads data meant for its identity because the identity's group is a member of the space holding it, and p2panda rotates that space's secret when a device is removed.

### The identity space and the personal space

`POST [@carbon]/identity` creates two spaces, like `/` and `/home`:

| Space | Holds | Who reads it |
|---|---|---|
| `identity:alice` | The identity's origin, device public keys, later public proofs, and one ref, to its personal space | The identity's devices; publishable |
| `personal:alice` | Refs to the spaces the identity creates or joins, device names and tags, and personal data | The identity, plus anyone it adds |

- **Put nothing in the identity space that you would not publish.** Privacy comes from nesting under the personal space, so companies and teams hang from personal, not from identity.
- **Device public keys stay in the identity space** because public proofs need them. A proof such as "I own this GitHub account" is signed by a device, and an outsider verifies it by tracing that device back to the identity's origin through `[device]/origin` entries. That reveals how many devices the identity has. Their names and tags, the inventory, stay in personal as `[device]/ref` entries.
- **The ref from identity to personal** is how a new device finds personal. It reveals only that a personal space exists.
- **Other identities may join a personal space,** such as a caretaker or someone holding power of attorney, without being conflated with the identity. Refs grant nothing, so they still need access to each space a ref points to.
- **The personal space's services** are named when the identity is created, since a space's services are fixed at creation. `[space]` and `[device]` always run there.
- **`manage`:** the identity space lists `self`, and the personal space lists the identity.

### Publishing an identity

- Private by default. `GET [@carbon]/identity/export` returns the identity space's ops with their CarbonDB signatures, and the app decides out of band where to post them, such as a website or nostr.
- An export verifies offline: CarbonDB signs its own envelope, separate from p2panda's signature over ciphertext, and the chain runs from the origin, whose hash is the identity id, through each device that authorized the next.
- A manual export shows the chain as of when it was made, so it misses later revocations. Choosing manual export means accepting that for privacy.
- `publish = true` under `[identity]` in the TOML file serves the identity to anyone who subscribes. The likely mechanism is a plain signed p2panda-core log on its own topic, outside p2panda-spaces (see [the backlog story](backlog/public-identity-proofs.md)).
- People who share a space with the identity can already see its device keys through its p2panda group. Publishing does not duplicate that.

## Members, access and manage

- p2panda owns the member list and answers who is in a space and at what level. CarbonDB has no member table or reconciler.
- CarbonDB records only what p2panda cannot hold: manage lists, removal cutoffs, invitations and device tags. Company policy is left to services.
- The calling device makes a p2panda change once, and a replicated `POST [space]/members` is the ordered event that triggers fire on.
- Membership decides access only. When a device syncs is set separately, and what a member may call is decided by roles in the app's own service, kept current by triggers on `POST [space]/members`.
- Members get `write` unless `POST [@carbon]/members` passes another access level.
- Pull members hold a space but receive no group secrets, so they cannot read it. Read and Write apply to a whole space, so a member holds every service in it: peers can enforce write limits by re-running handlers, but not read limits.
- A space is a document-level boundary whose members all see its full state. Data that must stay partitioned inside an organization is not deployed as a shared space.
- `manage` is CarbonDB's own permission, set at creation and carried on the origin op. It maps onto p2panda's Manage, which p2panda checks against direct members, so each managing device must be a direct Manage member of each space.
- `manage` defaults to the calling identity. An identity space lists `self`, because its id does not exist until its origin ops are written, and its devices need manage there to write its ref to personal.
- An empty manage list means no one may delete the space or add members, and the space dissolves when all members leave.
- A manage entry naming a company is also recorded in the child as the identities it expands to, because a handler cannot read another space deterministically.
- A device without the `manage` tag cannot write to `[space]`, so it cannot join a space on its own.
- A device holding the `manage` tag gets p2panda Manage in every space whose manage list names its identity. When a device is given the tag, or added with it, a device of the identity that already has manage grants it Manage in each of those spaces. Removing the tag revokes it the same way.
- RBAC, such as an app's role service, is an optional module. Without it a space still has strong identity and p2panda access levels, only no per-service-key permissions.

## Invitations

- What carries an invitation is purposely undefined. An invitation is a self-contained value that any medium can carry, and for now that means a QR code or a copied code. There is no inbox.
- A contact holds the identity's p2panda group id and how to reach one of its devices.
- An invite code holds the space id and how to reach the inviting device.
- `POST [@carbon]/members` takes a contact and returns a code. `POST [@carbon]/join` takes the code.
- Joining subscribes to the space, which announces the device's current key bundle, so bundles need no separate refresh.
- A manage device then adds the member in p2panda and writes `POST [space]/members`. It must come online at some point after the join, and the joiner's access stays empty until then.
- Joining writes `POST [space]/ref` in the calling space, unless it already holds one.
- Space membership must not be revealed to anyone who asks, so CarbonDB must not rely on subscribing to a space to learn who is in it.
- Relay servers are node configuration and need no design for the first draft.

## Removing a member

The call is not defined yet, but these rules hold:

- p2panda discards a message whose author has lost write access by the time it is processed, so without a rule a message concurrent with a removal could survive on one replica and vanish on another.
- The removal op carries the highest op the remover's device has seen from each device of the removed member. Every replica discards that member's ops above the cutoff, and rewind cleans up what depended on them.
- Discarding a removed member's unseen offline work is required behaviour.
- Removing someone from a company removes them from its teams, through `cascade`.
- A replica that keeps then discards an op shows it as rejected, while one where p2panda dropped it never holds it. Op history can differ though state matches.

## Sync, status and partitions

p2panda syncs only spaces a node has subscribed to, so when to sync is CarbonDB's choice:

```toml
[sync]
mode = "ref"   # the default; or "all", "manual", or "none" for single-node
```

- `ref` syncs a space the first time a ref reaches it, and from then on.
- `POST [@carbon]/sync/start` and `POST [@carbon]/sync/stop` start and stop syncing, and `DELETE [@carbon]/data` deletes this device's data for the space. p2panda has no per-space delete, so its store keeps the encrypted ops.
- `.ref()` returns a handle straight away.

`GET [@carbon]/status` never waits on the network:

| p2panda state | `sync` | `access` |
|---|---|---|
| Not subscribed | `off` | Last known, or `undefined` |
| Subscribed, space has not arrived | `waiting` | `undefined` |
| Arrived, not a member | `on` | `[]` |
| Arrived, member | `on` | The level, in a list |

It also returns the space's metadata and `peers`, the peers this device has synced this space with and when. It cannot know about peers it has never met. p2panda's `created` resolves for a non-member once the space arrives, and `joined` never resolves for one, so neither can be awaited without a timeout.

During a partition, GETs read the local copy, and writes apply locally and publish later if the last known access allows them. If the member was removed meanwhile, the cutoff discards them. A call on a `waiting` space is rejected.

## Core and runtime

**The core (Rust)** owns the p2panda nodes and spaces, identity and devices, the op envelope and clock, the shared order, SQLite execution, sessions, undo, rewind, the apply loop, routing by service name, and `[@carbon]`. It ships `[space]`, `[identity]` and `[device]` as packages written against the service API, like any other service: the core reads and writes their records only through calls to them, but for one exception: where an app needs an answer at once, as a handle finding a ref, the core reads a service's tables directly, namespaced, read-only and over committed state. It also keeps what an app needs from CarbonDB itself, such as watches and the device an identity acts from, so the runtime holds no CarbonDB state. It knows nothing of app schemas, routes, zod, roles or merge strategy.

**The service runtime (JS)** owns the entry point of each npm service: its route matching, the handler context and its access checks. How a service shapes its tables, such as by migrations, is the service's own concern.

CarbonDB is a message bus that handles state management: handlers and triggers subscribe to service keys, and handlers are language agnostic.

### Triggers

`POST [space]/trigger {to, in, out}` registers a trigger on the service key `to`. It is an op on `[space]`, so it needs manage and every replica holds the same triggers.

- A trigger is pure JSON with no handler. It forwards the triggering call's input to each key in `in` and its output to each key in `out`.
- It cannot change state itself and cannot make the triggering call fail. Any state change belongs to the services it forwards to; translating the input is the receiver's job.
- It fires only when the triggering call succeeds.
- The forwarded calls run in the applying transaction on every replica, so they rewind with the op that fired them.
- MicroQL-style references, such as `{id: '$in.identity'}`, could add mapping later without leaving JSON.

### Handles

Spaces and handles are one to one. `carbon` is the system handle, and every other space is reached through refs, which also sets the acting identity:

```js
const sales = carbon.ref('identity:alice').ref('personal:alice').ref('company:foo-corp').ref('team:sales')
await sales.call('POST [chat]/messages', {text: 'lunch?'})
```

The stories' names stay: `configure`, `services.add`, `createIdentity`, `resumeIdentity`, `joinIdentity`, `addDevice`, `listIdentities`, `createSpace`, `listSpaces`, `ref`, `call` and `watch`. They are thin wrappers over the calls in [services.md](services.md).

## Porting existing services

Notes on porting one particular platform are in [perspectives/brandon/porting-otto.md](../perspectives/brandon/porting-otto.md).

- Handlers that check current state, such as reading a record then answering not found, become local UX, not enforcement. Business rules run on every replica.
- Models written the traditional way work with last-writer-wins defaults, and schema metadata can optimize them, so adoption is progressive.
- A runtime that builds queries from data can narrow read sets to rows and derive ids and timestamps from the op.
- MicroQL keeps no state across runs beyond caching: services own persistence, and MicroQL can use guarantees they make but does not supply them.
- Dash Chat is a working precedent for chat on p2panda, with message, edit, delete and reply ops, receiver validation and a store-and-forward mailbox server.

