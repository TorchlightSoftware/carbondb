# CarbonDB first draft: implementation notes

Status: agreed — single-node built (build steps 1 and 2): the whole app interface, each service in a space of its own (one SQLite file with its op log and results), channels naming their services, calls between services and other effects made once and recorded on the op's log row, watches in the core and call timeouts; rewind and replay built on a single device (step 4, first half): the hybrid logical clock, rows received from other devices, the in-place check, rewind within a service and effects replayed from the row, described in [docs/replication.md](../../docs/replication.md); p2panda is next

This is everything we agreed that the code has to honour, gathered in one place for whoever builds the first draft. The nine numbered stories say what a person can do, [services.md](services.md) lists the builtin calls, and [oplog-examples.md](oplog-examples.md) shows them in use. This file covers what happens underneath. Where older notes disagree with it, this file wins.

The spec is the app interface: what an app calls and what CarbonDB guarantees in return. Everything beneath it may change as the build-out needs, as long as that interface and its guarantees hold. Sections after [Remote access stays outside CarbonDB](#remote-access-stays-outside-carbondb) describe replicated mode unless they say otherwise.

## Build order

1. **Single-node CarbonDB:** the Rust core behind napi-rs, with no p2panda, delivering the whole app interface below. Ops already go through each service's OpWriter, op log, pending ops and results, the same path synced ops will take, and each op's session changeset already says which tables it wrote.
2. **[chat] on it:** `main()` loads CarbonDB, registers [chat] with `carbon.services.add('chat', chat)` and reaches its channel through refs. [chat] stays small, resembling an ordinary backend service with minimal dependencies, with plain SQL and no build step.
3. **A small JS HTTP adapter** in front of [chat]. Not built: the tests call a channel handle directly, and an adapter is app code (see [Remote access stays outside CarbonDB](#remote-access-stays-outside-carbondb)).
4. **Replicated mode:** rewind and p2panda spaces, switched on by config with no change to the app. Rewind and replay are built on a single device ([docs/replication.md](../../docs/replication.md)); syncing over p2panda is planned in [p2panda-sync.md](../p2panda-sync.md).

The hello world is create a channel, add [chat] to it, post, edit, delete, list and watch, on single-node CarbonDB. Adding a member and joining ([story 7](07-add-members.md)) arrive with replicated mode. Richer service runtimes come later on the same app interface.

## The app interface

- **Setup:** an app's `main()` loads CarbonDB with its TOML config, registers each service with `carbon.services.add(name, impl)`, where `impl` is the service's exported module (its `manifest`, `setup` and `calls`), and gets the `carbon` handle. Whether it runs single-node or replicated is a config setting, so switching is a config change, not a refactor of the app.
- **Handles:** one per channel, reached through refs from `carbon` (see [Handles](#handles)).
- **`call(key, input)`:** runs the handler of the service the channel names in the key and resolves with its result, or rejects with a service error. A GET answers at once; a write once this device has applied it and the watches its commit owes a run have run again.
- **Timeouts:** every call must settle within its service's `timeout`, or it rejects with a `504`.
- **`watch(key, fn)`:** re-runs a GET and hands `fn` the result after each commit that touched what it read, and returns a function that stops it.
- **Service errors:** a rejection carries an HTTP-style status, a code and details, preserved across the core boundary (see [Service errors](#service-errors)).
- **Handlers** receive `path`, `input`, `env` (the service's `sql` binding, under the name its `service.toml` gives it), `call` and `effect`, and `op` when the call writes. `op` carries the key, path, input, the verified author and device as plain data, the time, `id()` and `random()`. `call(key, input)` makes a call inside this one, or to another service the channel names, as a message (see [Applying an op](#applying-an-op)); `effect(name, input)` makes any other effect.

The host function handlers use for SQL is internal and is not part of the spec.

### Single-node mode

```toml
[sync]
mode = "none"   # single-node: no p2panda node is started
```

- The same app interface and builtin services, on the Rust core with no p2panda and no network.
- Identities, channels, services and refs exist locally, so handles reach channels the same way.
- The op log is kept from the first draft, so local calls and synced ops share one apply path from the start.
- Effects this device can make alone run: calling a service it holds, and creating a new channel's or service's space. Effects that need peers have no handler and fail with `503 UNREACHABLE`: `POST [channel]/members`, `DELETE [channel]/members/:identity`, `DELETE [channel]/invitations/:invitation`, `POST [channel]/join`, `POST [channel]/leave`, `POST [channel]/manage`, `POST [identity]/devices`, and `POST [@carbon]/sync/start` and `/sync/stop`. `POST [@carbon]/identity` with a `code` fails with `501 SINGLE_NODE`.
- `GET [@carbon]/status` answers `sync: 'off'`, `peers: []`, and `manage` access for a channel the acting identity manages, `write` otherwise, since nothing else answers who has access.
- A single-node instance cannot switch to replicated later. Its data does not carry over.

### Service errors

- A service error is an Error with a numeric HTTP `status`, a string `code`, a message and details. The core guarantees that one thrown in a handler reaches the caller of `call` with status, code and details intact, so an adapter maps it to a response in a few lines.
- MicroQL's errors (`src/common/error-catalog.js` in the microql repo) already have that shape, as do those of other service runtimes we use, though their codes differ.
- The shared shape lives in `@tlsft/service-error`, beside `@tlsft/service-key`, and services throw its errors, such as `notFound` and `forbidden`. Reconciling the existing codes can follow.

## Remote access stays outside CarbonDB

CarbonDB does not bind services to HTTP, gRPC or any other protocol. An app that wants HTTP writes an adapter that turns a request into `channel.call(key, input)`, and a service error into a response: a request-to-response function, with something else doing the listening.

Why the boundary sits here:

1. **Serving a remote caller means deciding who that caller is.** That is authentication and RBAC, which CarbonDB leaves to optional services, such as an app's role service. CarbonDB knows only its members' signed identities.
2. **Channels have no network address.** Channels nest and act as virtual networks, with no tie to a transport, a network boundary or an IP address. Binding one to an external address carries assumptions CarbonDB should not make, so the adapter decides which channel a request reaches.
3. **Not every service key is HTTP.** The HTTP method in a service key is optional, and services may be local-only or served over gRPC.

The long-term case still fits: peers who should call a channel's services remotely without holding their data get finer read control than replication allows. Approving their identities and serving them is app or runtime work, not CarbonDB's. HTTP between services is set aside for now; a p2p service is never reached over HTTP as a fallback.

## Prove these first

None of this has run on real p2panda nodes yet. Each item is a design risk for replicated mode, so test it early:

- The invitation flow in [story 7](07-add-members.md): the invitation key added as a member, the bearer joining as it at once, a manager's device later replacing it with the invitee's devices, and the new member reading earlier messages.
- rusqlite 0.39.0 beside a live p2panda node in one binary. So far it has only compiled and inverted a changeset beside sqlx-sqlite 0.9.0 in a scratch crate.
- Each of a person's devices added directly to each service's space in a channel, including a device added after the channel was created, and the cost of that as channels hold more services.
- Ops at or below a removal cutoff reaching every replica (see [the backlog test](../backlog/verify-replica-op-sets.md)).
- Rewind cost under realistic load: ops per service, how deep a late op lands, changeset storage and apply latency. Performance claims are measured, not estimated.

## Stack

- **Packaging:** one native binary per platform. JS apps drive it as a library, and the Node binding is a napi-rs native module. The core runs on a thread of its own, driven by a single-threaded tokio runtime.
- **Handlers:** every service runs as a WebAssembly component on the core's WASM host, wasmtime, whatever language it was written in, and reaches the core only through the `carbondb:service` world ([docs/wasm-services.md](../../docs/wasm-services.md)). A JavaScript service's routes stay in its own module, compiled into its component with QuickJS. Validating a call's input is the service's own business: a handler that validates throws `validationError`, and CarbonDB never sees its schema.
- **SQLite:** real SQLite in the Rust core. A handler's `sql` binding offers `run`, `get` and `all`, each one statement with its parameters, run by the core in the call's transaction.
- **rusqlite:** pinned to 0.39.0 with the `session` and `bundled` features. Version 0.41.1 cannot link beside sqlx 0.9, because it needs libsqlite3-sys 0.39 and sqlx-sqlite 0.9.0 needs below 0.38.
- **p2panda:** provides transport, signed per-author logs and spaces, so sqlite-sync and cr-sqlite are not needed. Only capture and merge are ours.

## Files on disk

- The TOML file names the system database, which holds the identities on this device with their secret keys, the system log, and the spaces this device holds: for each, the package that runs it, the name its channel calls it by, and its channel. `configure` reads it once, and no other call takes a directory.
- Each service's space, a channel's own `[channel]` included, is one SQLite file beside the system database, named by its id.
- Each identity has its own p2panda store file beside it, because a p2panda node takes one set of credentials and one machine may hold several unlinkable identities. That means one p2panda node per identity on a device.
- A service's op log and results live in its own file, beside its tables, so they commit atomically with its writes. The session records every table but CarbonDB's own, using rusqlite's `table_filter`.
- The core's own tables, named `carbon_` (the op log and results in every service's file, and the identities and spaces in the system database), move forward by migrations, tracked in `carbon_migrations` and applied in one transaction when the file opens. The system database's file applies a service's migrations, then its own. Until 1.0 these migrations may be consolidated, and development databases deleted when they are.

## Channels and services

- **system** is the root, local and never replicated. It holds what must be readable before any identity opens. It is a plain local log, not a p2panda space, because a p2panda space needs a node and a node needs an identity. `carbon` is its handle, and only `[@carbon]` answers there.
- **A service** is the code: a package, as an actor type or a Durable Object class is. Each service in a channel is one instance of it, running in a space of its own: one p2panda space, one SQLite file, one op log. Many instances run the same package, such as a [chat] for a company and another for its marketing team; how many is a data custody decision made per use case. "Space" names only this implementation, never a domain term.
- **A channel** names the services that work together, `[name]` to service, as candle-connect maps a name to a host in Otto, and holds the people given access to them. A call on a channel's handle resolves `[name]` through the channel. A service belongs to one channel.
- **Names:** a channel is named by its kind and a lowercase kebab-case slug, such as `identity:alice`, `company:foo-corp` and `team:sales`. The root is `system`.
- **Kind** is a free label for apps and interfaces, `channel` by default. CarbonDB gives meaning only to `identity` and `personal`, the kinds it creates itself.
- **Local services,** marked `[@name]`, run only on this device and never replicate. Replicated services can never call them (a 403), because that would duplicate their effects on every replica.
- **`[@carbon]`** is the core's local service, implemented in Rust because it reaches keys and p2panda. It answers only on `carbon`: founding and listing identities, and the status, sync and deletion of this device's copy of a channel or service, named in its input. Its ops go in the system log, with the effects they made; what it changes in a channel it changes through that channel's own calls, made as effects, each an op there that replicates as itself.
- **`[channel]`** runs in every channel's own space: its origin, its services, its refs, its members and invitations, and its manage list.
- **`[identity]`** runs only in an identity's own channel: the identity's origin and its devices, with their names and tags. An app may name its own service `[identity]`, such as one holding roles, in another channel, since names resolve per channel. A package's registered name is still device-wide, so an app registers it under another name, such as `roles` (see [packages.md](packages.md#a-channel-names-its-services-packages)).
- **Calls between services:** a call to the same service runs inside the op, under a savepoint. A call to another service the channel names is a message: the `call` effect, made on the op's first run, which posts it to that service as an op there and records its answer on the op's log row. Every replay gets the recorded answer.

### Creating a channel

1. `POST [channel]/channel` runs on the handle of the channel it nests in, which points to the new one with a ref. Like every `[channel]` POST, it needs `manage` there, so you cannot create a channel inside one you do not manage.
2. Its handler derives the new channel's id from the op and the name, the same on every replica, and makes two effects: `create`, the channel's space, running `[channel]`; and `call`, its first op there, `POST [channel]/origin`, carrying its kind, name and manage list. It repeats the create call's fields because the channel's members may not be able to read the channel it was created from. Then the handler writes the ref. Whether p2panda assigns the id once replication is built, as was agreed for spaces, or takes the one the op derives, is not yet defined. Recognising a channel an interrupted call already created is left to [resuming after a crash](../backlog/resume-after-crash.md).
3. `POST [channel]/services` adds a service the same way: `create`, the service's space, running a package registered on this device; then the handler records the entry, its name, id, package and version. Names are unique in a channel, and `channel` is reserved. A manager may add services at any time.

Creating a channel or a service cannot be undone: its effects are made once. If a rewind refuses the op at its place, the channel or service stays, unattached. The [security review](../backlog/security-review.md) revisits this.

### Rules on `[channel]` and origin ops

- Every POST to `[channel]` but its origin requires `manage` on the channel, except `POST [channel]/leave`.
- An origin op can be written only once: `[channel]` once per channel, `[identity]` once per identity.
- `POST [channel]/seal` makes the channel refuse every later POST, the same on every replica. An identity's own channel is sealed at founding.
- A call to a service the channel does not name is rejected with a 404.
- Removing a service from a channel, deleting a channel, and moving a service to a new version ([backlog](../backlog/upgrade-package-version.md)) are not yet defined.

### Hierarchy

- Channels do not nest in p2panda: each service is its own p2panda space, and adding a group as a member gives everyone in it access, which is not what a parent and child mean.
- Hierarchy lives in refs and manage lists. A ref only points and grants nothing, and a channel may have many refs.
- Parenthood is set at creation. Reassigning it is undefined (see [the backlog story](../backlog/reassign-space-parent.md)).
- An employee identity reaches a team channel by first looking in its company, because that access comes through the company. Only a direct relationship, such as a consultant, puts a team ref directly in a personal channel.
- Reaching a channel through refs needs the refs to exist and the identity to have access. What is possible in each channel depends on that access.
- Adding someone to a child channel does not add them to its parent. But it may be prevented based on RBAC in the parent (see [the backlog story](../backlog/parent-rules-on-child-members.md)).
- `cascade` on a ref takes `remove`, where losing access to the parent removes access to the child, and `delete`, where deleting the parent deletes the child, given manage rights there. Teams within a company use both.

## The op

- An op is a service call: service key plus fields, plus op hash, device (signs and syncs), author (the person, for access control and audit), hybrid logical clock time and version: the hash of the package that handles it.
- For now, a device that does not run the version an op names refuses that op (see [old ops](../backlog/old-ops.md)).
- The author comes from the signed envelope, never from a field on the call.
- The op hash is computed over CarbonDB's signed envelope before the handler runs. An op's effects are not part of its call, so the hash does not cover them. It is separate from the p2panda operation hash, which does not exist yet at that point and covers ciphertext.
- Ops relate through the hybrid logical clock, not through lists of other ops. The one exception is the per-device removal cutoff.
- CarbonDB treats an op's input as opaque data. A handler error, including the service's own input validation, rejects the op.
- The op log, aligned with p2panda, is the service's audit log.
- GET is never an op. PUT and DELETE are idempotent and scoped to the path they name, and POST and PATCH promise nothing unless tagged. A POST may carry an optional tag such as `append`.

## Applying an op

- Every replica of a service sorts its ops by (hybrid logical clock, device, op hash), drops duplicates, and re-runs the service's handler in that order, so business rules give the same result everywhere.
- The core owns the order and the changesets, so it decides when a handler runs or re-runs. It routes every call by the name the channel gives the service to that service's entry point, the same for every package, whatever language built it, and the entry point settles it once.
- The op log records what this device received, local calls and synced ops alike. A service's OpWriter writes each op as it arrives, before it applies, as a durable queue would, so a crash loses nothing. While an op applies, its transaction holds the file, so an op arriving meanwhile is held by the OpWriter and written as soon as that turn ends, before the next op applies. Rows are never changed, except that an op's effects are written onto its row when its first run settles.
- Results record what this device did with each op: applied or refused with its service error, with its output. A result is keyed by its op hash and never leaves this device. A rewind replaces results; it never touches the log.
- The pending ops are the op log minus the results: the ops with no result yet. They are read from the service's file each turn, never kept in memory, and apply one at a time in the shared order, so ops left without a result by a crash simply apply next time the service opens. A local call is stamped when it arrives, since that is when it is written; an op that arrives later but sorts earlier is handled by rewind.
- Ops apply one at a time per service, and services apply side by side. A service is the finest unit: its tables, op log and results share one SQLite file, which has one writer, and an op holds its transaction while its handler awaits.
- Every op takes the same path, local or synced: the OpWriter logs it, it applies in turn, and its result is recorded. Nothing is refused before it is logged, so every failure is reported, audited and kept in the history the same way.
- Pending ops apply only once the service's package is registered on this device; until then the service is initializing, and its ops and their callers wait. An op made for a package this device has not registered is refused with a 404 when its turn comes, before any handler sees it; that refusal keeps it from running again and from being published.
- The callers waiting on an op are kept beside it in memory, by op hash, and answered when its result is recorded. An op resumed from the log has none.
- Each service has a RunState: initializing, with its package not registered here yet; idle; applying an op; or stopped. The app can read it and watch it.
- A call a handler makes to its own service runs inside the same op, under a savepoint: it sees the writes before it, and when it fails only its own writes roll back and the error reaches the handler that made it. A GET inside an op reads through the op's transaction. A call to another service the channel names is a message, made as the `call` effect (see [Effects](#effects)).

### Effects

What a handler does outside its own service's tables is an effect: calling another service, creating a channel's or service's space, changing p2panda membership, a handshake with another device. How they work, precisely, is in [docs/replication.md](../../docs/replication.md#effects-are-made-once).

- An effect is made once, on the op's first run, on the device that made it. Its output, success or failure, is recorded on the op's log row, in the order the handler asked for it, when the first run settles, and travels with the op. The core never interprets an output; the service decides what is a failure.
- Every later run, on another device or after a rewind here, gets the recorded outputs in order instead of making the effects again. A run asking for more effects than the first run made gets an unreachable failure, the same on every replica.
- A service declares its effects in its `service.toml`, `[effects.<name>] replicate = "never"` (the default) or `"always"`, which makes the effect again on every run on every replica, never recorded.
- The registry holds each effect's handler by name. Single-node, this device makes what it can alone (`call` and `create`); an effect that needs peers has no handler and fails as unreachable until p2panda.
- A call's answer is what the other service answered on this device at that moment; a late op there can still change its outcome, and if the caller then fails, what the other service did stays. Only delivery is guaranteed between services, and atomicity ends at the service. Report-back is rejected: a caller that needs the current answer asks again.
- A call to a service this device holds no copy of that it can run, being pull-only or not a member, fails as unreachable, the same on every run.

The steps for each op:

1. The core opens a transaction on the service's database, with a session recording its changes.
2. It hands the call to the service's entry point and waits for it to settle. Handlers may be async.
3. Database calls from the handler are synchronous and run inside the open transaction.
4. When the entry point settles it with a value, the core records the op's result and its effects, and commits, taking the session's changeset, which it keeps with the result. When it settles with an error, or its deadline passes first, the core rolls back and records the op's result as refused, with its error, and its effects.
5. The core keeps every watch: a GET, the tables it read, and when it must run again after a commit that changed one of them, or after its service stops or resumes. A write from the app is answered once the watches its commit owes a run have run again: those that read a table it wrote, and those in flight when it landed, which are judged when they answer. A GET from the app answers at once. The runtime only registers watches and hands each answer to the app, and apps get no separate event API.

Timeouts and refusals:

- A call refused on the author's device is not published.
- A rejection on a later replica leaves the op in the log; its result records the refusal, on that device only, and `[@error]` reads it (see [rejected after merge](../backlog/rejected-after-merge.md)).
- Every op stays in the log, so no data is lost. Each service gets a standard op history interface for finding a culprit op, such as a destructive offline edit, and reverting it. A revert is a new compensating op.
- A handler that overruns a timeout rolls back. Each service names its timeout in its `service.toml`, 200ms by default, and one deadline covers a call and the calls it makes. The core has no timers, so the host schedules the deadline, and the core settles the call with a `504` when it passes and drops its handler's later answer. A local call is then refused and never published. A synced op is not recorded as rejected, because a timeout differs between replicas, so the service pauses, reports the error and retries. Not built.

System failures:

- A system failure is SQLite failing in itself, not the op: an I/O error, a full disk, the file busy or locked by another process, corruption, or memory. The store classifies them by SQLite's error code on every statement, a handler's own included, and keeps the first it meets, so a handler that catches the error cannot hide it. No statement waits for a lock.
- Writing an op to the log or its result is never the op's own failure: when it fails the service stops too.
- A stopped service neither logs nor applies ops. The op that was applying rolls back and stays pending, every caller waiting on its ops gets a 503 carrying the failure, and so does every new call and every watch. It stays stopped until the problem is fixed by hand and the service is resumed, likely from a button in the app; then its pending ops apply from the first. A failure stops only the service it hit.

The runtime checks access while applying an op, against the service's state at that op's position in the shared order, so a late role change re-runs what read it. `[channel]` checks manage from its own origin, not from p2panda, because p2panda's current state differs between replicas.

### Reads

GETs and watch re-runs use a second read-only connection in WAL mode, so they see committed state and cannot write. They answer from this device's copy, even when it is behind other members. SQLite's authorizer reports what a GET reads, and a watch re-runs when a committed op's changeset changes a table it read. A batch of synced ops re-runs a watch once.

## Late ops and rewind

- Each op's writes are recorded as a SQLite session changeset, which is both its undo record and its write set. The authorizer gives its read set by table, narrowed to one row when a statement looks it up by its whole key ([docs/replication.md](../../docs/replication.md#reading-one-row-by-its-key)).
- Rewind lives in its own module beside dispatch: it decides whether a late op fits in place and which later ops to undo; undoing a changeset is a store primitive. Each op's read set and changeset are kept with its result, never sent. How it works, precisely, is in [docs/replication.md](../../docs/replication.md).
- Taking the write set from the changeset, in single-node mode too, was chosen over the authorizer's record of written tables:
  - It is exact. The authorizer counts a table as written when a statement naming it is prepared, even one that changes no rows; a changeset holds only net row changes, so a no-op update or an insert deleted again re-runs no watch.
  - It is one record. What a watch treats as written is exactly what rewind undoes and checks for overlap, so they cannot disagree.
  - It costs a session on every op in single-node mode too, recording old values and reading each changed table's columns once per op. This is to be measured, not estimated.
  - The core's own tables, the op log and results, leave the write set, since the session leaves them out. No watch reads them.
  - A savepoint rolled back inside the transaction leaves nothing in the changeset, as the [session spike](../../spikes/session-rowid/README.md) shows, so a failed nested call neither re-runs watches nor reaches rewind.
- A late op is tried in place: it keeps that run when no later op wrote a row it read, no later op read a row it wrote, and no later op wrote a row it wrote.
- Otherwise its run rolls back, and every later op of the service that read or wrote any of its tables is undone, newest first, and its result forgotten; the late op applies at its place and they re-run in order, each tried in place, replaying the effects recorded on their rows. A rewind never leaves its service: what another service did is its own, and a message it was sent stays sent.
- The `append` tag is an author's promise that no handler depends on the row being absent. A late tagged op applies in place and re-runs nothing. Not built: inserts of different rows already apply in place by themselves, and today an insert still rewinds the later ops that read its whole table.
- The core treats tags as opaque flags. A tagged op whose changeset holds anything but inserts is rejected on every replica. A wrong promise about reads is not detected at runtime; the test harness catches it.
- A row's key comes from the op that creates it, never from the rows already in the table: a key SQLite assigns, a rowid or an AUTOINCREMENT id, depends on those rows, so after a rewind it can move under the row's author. After a service's `setup`, the core checks every table it owns has a primary key that is not SQLite's rowid, and refuses the service with an error naming the table, what to change and why. Handlers take keys from `op.id()`, or an integer key from `DEFAULT (op_id())`, a SQL function every runtime registers that draws below 2^53. Sessions do not record schema changes, so schema changes only in `setup`, outside any op.

## Publishing

- Each service's op log is its outbox. A publisher drains this device's ops whose result applied, with their recorded effects, also after a crash; a call refused on its own device stays in the log unpublished, its result saying why. Duplicates drop by op hash. The system log is never published.
- p2panda streams replay unacknowledged operations after a restart, so the core acknowledges after commit to cover a crash on the receiving side.
- A call resolves once this device has applied it, without waiting for other members.

## Repeatability

Every state change belongs to one class, as in the table in [README.md](README.md#repeatability-of-state-mutations):

- **Undoable:** SQLite writes in the service's own tables, undone and re-run by rewind.
- **Repeatable, grow-only:** such as content-addressed blob puts. No undo is needed.
- **Irreversible:** such as calling another service, p2panda removal, creating a channel or a service, or sending email. These are effects: made once, on the op's first run, and replayed from the row after.

## Rules for service authors

- All mutations go through the op log; nothing mutates a table directly.
- SQLite state is a pure fold of valid ops, and can be thrown away and rebuilt.
- Handlers take time, ids and randomness from the op context, never from `Date.now`, `randomUUID` or `Math.random`. The author picks ids and timestamps, and they travel in the op.
- Rules needing global knowledge, such as uniqueness or a non-negative balance, are modelled as visible conflicts or resolved deterministically. A value that must be unique is checked with a query, never left to a constraint.
- A row's key comes from the op that creates it: `op.id()`, or `DEFAULT (op_id())` in a WITHOUT ROWID table.
- Data that must change together in one op lives in one service: atomicity ends at the service. A call to another service is a message whose answer may later change there; handle a failed call as a saga does. How the service-author guide teaches this is open.
- Anything outside the service's own tables is an effect, declared in its `service.toml` when it is not `call`.
- All implementations of one service must behave identically, or replicas drift silently. This is a documented rule, not a mechanism.
- Traditional models work with last-writer-wins from the shared order; better mergeable data structures come from schema design.

A test harness runs the same ops on two fresh databases in different delivery orders and compares state. There is no runtime detection.

## Identity and devices

- An identity is a channel running only `[identity]`, plus `[channel]` for its one ref, to the personal channel. It is shared only by the person's own devices, never with anyone else, and nothing nests under it but the personal channel. It is sealed at founding: `POST [channel]/origin`, its ref to the personal channel, then `POST [channel]/seal`, after which its `[channel]` refuses every op, the same on every replica.
- `[identity]` holds the identity's origin and its devices, each with its public key, name and tags, private to the person's devices. Only the identity itself adds, renames or retags its devices.
- The identity id is the hash of `POST [identity]/origin` with the first device's public key, signed by that device, so it stays the same as keys rotate.
- `POST [identity]/devices` adds a device: the handshake with it is an effect, and the op records its key, name and tags. Enrolling adds the device to every service of the identity's own channel, its `[channel]` included, as a p2panda membership change, which the seal does not stop.
- p2panda's intended shape is an identity as a group of its devices, added to a space once. p2panda's high-level API refuses write through a group, so for now CarbonDB adds each device to each service's space directly ([upstream group write](backlog/upstream-group-write.md)), and a member's new device needs a manager of each shared channel online ([member devices](backlog/member-devices.md)).
- p2panda does not implement Manage for a group, so CarbonDB's own manage, per identity, maps onto each device acting for it.
- How a peer in a shared channel ties a device to its identity, without reading the identity's own channel, is not yet defined; it was the p2panda group before devices were added directly.
- The paper key is an `[identity]` entry only, and stays out of p2panda until recovery is designed ([backlog](../backlog/paper-key.md)).
- Device additions commute, so this log needs no rewind for the current stories.
- Prefer Keybase semantics unless there is a well-justified reason to diverge. Having no server is one such divergence.

### No secrets in replicated ops

- Device private keys stay in the system database. Group secrets belong to p2panda, which keeps them per device and sends them only encrypted to each device. Callers reach either only through `[@carbon]`.
- There is no per-identity key. A device reads a service's data because it is a member of that service's space, and p2panda rotates that space's secret when a device is removed. A pull-only device gets no keys: it carries ciphertext it cannot read.

### The identity's own channel and the personal channel

`POST [@carbon]/identity` creates two channels, like `/` and `/home`:

| Channel | Holds | Who reads it |
|---|---|---|
| `identity:alice` | `[channel]`: its origin and one ref, to the personal channel, sealed. `[identity]`: the identity's origin and its devices, with their keys, names and tags | The identity's own devices only |
| `personal:alice` | `[channel]`: refs to the channels the identity creates or joins, and the services it names, given at founding or added later; those services' personal data | The identity, plus anyone it adds |

- **The identity's own channel is never shared.** Companies and teams hang from the personal channel, never from the identity's own.
- **The ref from the identity's own channel to the personal one** is how a new device finds the personal channel.
- **Other identities may join a personal channel,** such as a caretaker or someone holding power of attorney, without being conflated with the identity. Refs grant nothing, so they still need access to each channel a ref points to.
- **The personal channel's services** are named when the identity is created, or added later with `POST [channel]/services`. `[channel]` always runs there.
- **`manage`:** both list the identity itself, whose id is known before its origin is written, since it is derived from the first device's key.

### Publishing an identity

Deferred with the public record of an identity's devices ([backlog](../backlog/public-identity-proofs.md)), and `GET [@carbon]/identity/export`, which is not built. The earlier design: private by default; an export of signed ops the app posts out of band, verified offline from the origin through each device that authorized the next; and `publish = true` under `[identity]` in the TOML file serving it to subscribers. Now that `[identity]` keeps the devices' private names and tags beside their keys, what an export holds is not yet defined.

## Members, access and manage

- Access is per channel: `pull`, `read`, `write` or `manage`. A member has that access on every service in the channel; per-service access is hard for users and administrators to reason about. A channel's members are the members of each of its services, so every peer of a service holds the same channel.
- A device's access to a channel's services is the lower of what its identity is granted on the channel and what its own policy accepts (`device:CloudflareRelay [pull]`), so users configure blind relays. Where a person sets a device's policy is not yet defined.
- `[channel]` records the channel's members, its pending invitations and its manage list, replicated so every member sees them. p2panda enforces access on each service's space; each change to it is an effect of the `[channel]` op, made once on the device that made it.
- Membership decides access only. When a device syncs is set separately, and what a member may call is decided by roles in the app's own service. Triggers on `[space]` were dropped; how a role service learns of membership changes is not yet defined.
- Members get `write` unless `POST [channel]/members` passes another access level.
- Pull members hold a service's space but receive no group secrets, so they cannot read it. Read and Write apply to the whole channel, so a member holds every service in it: peers can enforce write limits by re-running handlers, but not read limits.
- A channel is a document-level boundary whose members all see the full state of its services. Data that must stay partitioned inside an organization goes in another channel.
- `manage` is CarbonDB's own permission, per channel, set at creation, carried on the origin op, and extended with `POST [channel]/manage`. It maps onto p2panda's Manage, which p2panda checks against direct members, so each managing device must be a direct Manage member of each service's space in the channel.
- `manage` defaults to the calling identity.
- An empty manage list means no one may change the channel: add services or members, or delete it. The channel dissolves when all members leave.
- A manage entry naming a company is also recorded in the child as the identities it expands to, because a handler cannot read another channel deterministically. Not built.
- Devices carry tags, and an identity's first device is tagged `manage`. How a device's `manage` tag relates to its device policy, and to p2panda Manage in each channel its identity manages, is not yet defined.
- RBAC, such as an app's role service, is an optional module. Without it a channel still has strong identity and p2panda access levels, only no per-service-key permissions.

## Invitations

- What carries an invitation is purposely undefined. An invitation is a self-contained value that any medium can carry, and for now that means a QR code or a copied code. There is no inbox.
- What a contact holds is not yet defined.
- `POST [channel]/members` takes a contact and an access level and returns `{invitation, code}`. `POST [channel]/join` takes the code, on the channel that will point to the joined one, by default the personal channel.
- An invitation works through an invitation key: the inviting manager adds the key as a member when inviting, and the code carries its secret as a bearer token. The bearer joins as that key at once, and a manager's device later replaces it with the invitee's own devices. Cancelling an invitation, `DELETE [channel]/invitations/:invitation`, removes the key.
- Invitations are replicated in the channel: every member sees which are pending (`GET [channel]/invitations`), and a manager can cancel one.
- The code carries the proof every member's device checks an acceptance against, so validating it needs no manager online. Adding the invitee's devices in p2panda takes a device with manage on the channel, the same limit as a member's new device.
- An invitation need not carry the invitee's device keys: they are exchanged in the initial handshake.
- Joining writes a ref to the joined channel in the calling channel. Which op records the invitee as a member once they accept is not yet defined.
- Channel membership must not be revealed to anyone who asks, so CarbonDB must not rely on subscribing to a service's space to learn who is in it.
- Relay servers are node configuration and need no design for the first draft.

## Removing a member and leaving

- `DELETE [channel]/members/:identity` needs manage. It removes the member from the channel and every service in it: what they wrote before stays, and what they write after is refused.
- `POST [channel]/leave` needs no manage. The leaver's devices delete their copies. It is best effort: it takes the member out in p2panda if p2panda allows that without manage, and otherwise stops their devices at once and leaves the removal to a manager; what is built is reported back.
- p2panda discards a message whose author has lost write access by the time it is processed, so without a rule a message concurrent with a removal could survive on one replica and vanish on another.
- The removal op carries the highest op the remover's device has seen from each device of the removed member. Every replica discards that member's ops above the cutoff, and rewind cleans up what depended on them ([backlog](../backlog/remove-member.md)).
- Discarding a removed member's unseen offline work is required behaviour.
- Removing someone from a company removes them from its teams, through `cascade`.
- A replica that keeps then discards an op shows it as rejected, while one where p2panda dropped it never holds it. Op history can differ though state matches.

## Sync, status and partitions

p2panda syncs only spaces a node has subscribed to, so when to sync is CarbonDB's choice:

```toml
[sync]
mode = "ref"   # the default; or "all", "manual", or "none" for single-node
```

- `ref` syncs a channel, and each of its services this device runs, the first time a ref reaches it, and from then on. CarbonDB's core, in its init and maintenance loop, reads the channels its identity belongs to, joins their services' spaces and fetches their packages, at the lower of the identity's grant and the device's policy.
- `POST [@carbon]/sync/start` and `POST [@carbon]/sync/stop` start and stop syncing a channel or a service named in their input; for a channel, each of its services this device runs. `DELETE [@carbon]/data` deletes this device's copy of a service, or of a channel and its services. p2panda has no per-space delete, so its store keeps the encrypted ops.
- `.ref()` returns a handle straight away.

`GET [@carbon]/status`, naming a channel or a service, never waits on the network:

| p2panda state | `sync` | `access` |
|---|---|---|
| Not subscribed | `off` | Last known, or `undefined` |
| Subscribed, the service has not arrived | `waiting` | `undefined` |
| Arrived, not a member | `on` | `[]` |
| Arrived, member | `on` | The level, in a list |

It also returns the name and `peers`, the devices this one has synced with and when, and for a channel its kind and `services: [{name, access, sync}]`. It cannot know about peers it has never met. p2panda's `created` resolves for a non-member once the space arrives, and `joined` never resolves for one, so neither can be awaited without a timeout.

During a partition, GETs read the local copy, and writes apply locally and publish later if the last known access allows them. If the member was removed meanwhile, the cutoff discards them. A call on a `waiting` service is rejected.

## Core and runtime

**The core (Rust)** owns the p2panda nodes and spaces, identity and devices, the op envelope and clock, the shared order, SQLite execution, sessions, undo, rewind, the apply loop, the effect registry, resolving `[name]` through the channel, and `[@carbon]`. It ships `[channel]` and `[identity]` as packages written against the service API, like any other service: the core reads and writes their records only through calls to them, but for one exception: where an app needs an answer at once, as a handle finding a ref, the core reads `[channel]`'s tables directly, read-only, over committed state in that service's own file. It also keeps what an app needs from CarbonDB itself, such as watches, the spaces this device holds and the device an identity acts from, so the runtime holds no CarbonDB state. It knows nothing of app schemas, routes, zod, roles or merge strategy.

**The WASM host** runs each package's component: an instance per call, as a task on the core's thread, reaching only the world's host imports and a closed WASI. Inside a JavaScript component, `carbondb/service` owns the entry point: route matching and the handler context. How a service shapes its tables, such as by migrations, is the service's own concern. **The JS runtime** only passes the app's calls to the core and hands back its answers.

Services are actors: each holds its own state, and reaches the others only by message, with delivery the only guarantee between them. That also lets each service choose its own hosting and custody. Handlers are language agnostic.

### Triggers

Dropped. `POST [space]/trigger` was designed and never built. How one service follows another's calls is not yet defined.

### Handles

Channels and handles are one to one. `carbon` is the system handle, and every channel is reached through refs, which also sets the acting identity:

```js
const sales = carbon.ref('identity:alice').ref('personal:alice').ref('company:foo-corp').ref('team:sales')
await sales.call('POST [chat]/messages', {text: 'lunch?'})
sales.channel   // the channel's id
```

`carbon.ref('identity:alice')` reaches the identity's own channel from the system database; every later `ref` reads the channel's `[channel]` refs as committed, and throws a 404 for one that is not there. An app needs only `configure`, `services.add`, `ref`, `call` and `watch`. Everything else is a call, listed in [services.md](services.md), such as creating and listing identities, channels and services. A handle keeps no helpers over them; what a call creates is reached through `ref`.

## Porting existing services

Notes on porting one particular platform are in [perspectives/brandon/porting-otto.md](../../perspectives/brandon/porting-otto.md).

- Handlers that check current state, such as reading a record then answering not found, become local UX, not enforcement. Business rules run on every replica.
- A channel takes the place of Otto's service registry: Otto maps each name to a host, and a channel maps each `[name]` to a service, reached over p2p instead of HTTP.
- Models written the traditional way work with last-writer-wins defaults, and schema metadata can optimize them, so adoption is progressive.
- A runtime that builds queries from data can narrow read sets to rows and derive ids and timestamps from the op.
- MicroQL keeps no state across runs beyond caching: services own persistence, and MicroQL can use guarantees they make but does not supply them.
- Dash Chat is a working precedent for chat on p2panda, with message, edit, delete and reply ops, receiver validation and a store-and-forward mailbox server.
