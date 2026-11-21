# carbondb

Write a service the way you write an ordinary backend service, with named calls, SQL tables and handlers, and CarbonDB keeps its state on the devices of the people using it.

This release runs **single-node**: one device, no peers. The app interface is the one replicated mode will keep, so switching later is a config change, not a rewrite.

## Use

```js
import {configure} from 'carbondb'
import {component as chat} from '@tlsft/carbondb-svc-chat'

const carbon = await configure('./carbondb.toml')
await carbon.services.add('chat', chat)   // once it has compiled

await carbon.call('POST [@carbon]/identity', {slug: 'alice', device: 'laptop'})
const personal = carbon.ref('identity:alice').ref('personal:alice')
await personal.call('POST [@carbon]/channel', {name: 'Lobby'})
const lobby = personal.ref('channel:lobby')
await lobby.call('POST [@carbon]/services', {name: 'chat'})

await lobby.call('POST [chat]/messages', {text: 'hello'})
const stop = lobby.watch('GET [chat]/messages', (messages) => render(messages))
```

```toml
# carbondb.toml
system = "data/system.db"   # each service's file sits beside it
# compiled = "data/compiled" # where compiled components are kept; beside the system database by default

[sync]
mode = "none"               # single-node
```

## Writing a service

A service is a module that exports `calls`, keyed by service key, each with a `handler`, and an optional `setup(db)`, which sets up the service's schema on every open, inside a transaction, and finishes before it returns; beside it, its `service.toml`. It runs as a WebAssembly component, not in Node: `carbondb-build <service.js> <service.toml> <out.wasm>`, or `buildService({source, manifest, out})` from `carbondb/build`, bundles it with its dependencies, compiles it with jco's QuickJS backend, and embeds its `service.toml` in the component, its package. jco (`@bytecodealliance/jco`) is the service's own build dependency. A `.sql` file imports as its text, so a service keeps D1-style migration files, and `carbondb/migrations` applies them from `setup`:

```js
import {applyMigrations} from 'carbondb/migrations'
import createMessage from './migrations/0001_create_message.sql'

const migrations = [{name: '0001_create_message', sql: createMessage}]
export const setup = (db) => applyMigrations(db, migrations)

export const calls = {
  'GET [chat]/messages': {handler: ({env: {db}}) => db.all('SELECT id, text FROM message')}
}
```

Inside the component, a service reaches only its binding, its calls and effects, `fetch`, and a closed WASI: no files, sockets or environment. There is no Node inside the component: no `http`, `undici` or other built-ins, only the globals `carbondb/service` installs. `console` logs for whoever runs the device. In a write, `Date.now()` is the op's time and `Math.random()` draws from the op, so every replica's run sees the same; a GET sees this device's clock. `fetch` reaches the outside world as an effect: in a write, the request is made once and every replay gets its recorded response, read whole; in a GET, it is made every time. How the host runs components is in [docs/wasm-services.md](../../docs/wasm-services.md).

## The interface

- **`configure(path)`** loads CarbonDB from its TOML file and returns `carbon`, the system handle. The core runs on a thread of its own, and every service inside it as a component; this package hands it calls and hands the app its answers.
- **`carbon.services.add(name, package)`** registers a service on every start. `package` is a component carrying its `service.toml`, as `carbondb-build` builds one: a file URL, a path or its bytes. CarbonDB reads the `service.toml`, and refuses with a 400 what is not a component, a component without one, or one holding a field it does not know. The service runs under `name`, which overrides the name in its `service.toml`. Its version is the hash of the package's bytes. It returns a promise that resolves once the service has compiled, off the core's thread, and rejects a component that does not compile, leaving its name free. A JavaScript service is slow to compile the first time; its compiled code is kept on disk, so a restart loads it. Calls made to a service still compiling wait for it to start (see Lifecycle). A name already registered, CarbonDB's own included, is refused with a 409. Its `service.toml` may declare the effects it makes (see Handlers).
- **Schema:** a service keeps its tables in a SQLite file of its own, beside CarbonDB's record of it (its op log and results, named `carbon_`), which the service may not touch, nor attach another file. It writes any SQL SQLite accepts. A row's key must come from the op that creates it, never from SQLite: after `setup`, a table whose key is SQLite's rowid (`INTEGER PRIMARY KEY`, with or without `AUTOINCREMENT`) or that has no primary key is refused, with an error saying what to change. Use `op.id()`, or `id INTEGER PRIMARY KEY DEFAULT (op_id())` in a `WITHOUT ROWID` table for an integer key. A value that must be unique is checked with a query before writing, never left to a `UNIQUE` or foreign key constraint or `INSERT OR IGNORE`: devices apply ops in one shared order, and CarbonDB sees what a handler queried, not what a constraint checked, so a constraint deciding the outcome can leave devices disagreeing. Constraints may stay as a backstop that never fires. Look a row up by its whole primary key (`SELECT ... FROM note WHERE id = ?`, `UPDATE note SET ... WHERE id = ?`, `DELETE FROM note WHERE id = ?`) and CarbonDB records that the op read just that row, so an op from another device that changes a different row of the table makes it re-run nothing. Any other query (a search by another column, a join, a subquery, an `OR` or a range) reads the whole table: still correct, but any write to the table from a concurrent op re-runs it. A key of TEXT or INTEGER compared exactly, a value bound with the key's own type, and a table without triggers keep a lookup to one row. `carbondb/migrations` applies D1-style migrations from `setup` (see Writing a service).
- **Handles** reach channels, one per channel: `carbon.ref('identity:alice').ref('personal:alice').ref('channel:lobby')`. A handle acts as the identity its ref chain passed through. `ref()` answers at once, read from the channel's refs as committed, and throws a 404 for a ref that is not there. A call on a channel's handle, `POST [chat]/messages`, reaches the service the channel names `[chat]`.
- **`call(key, input)`** resolves with the handler's result: a GET at once, and a write once this device has applied it and every watch its commit touched, or that was running when it landed, has run again. It rejects with a service error from [`@tlsft/service-error`](https://www.npmjs.com/package/@tlsft/service-error): status, code, message and details intact.
- **Timeout:** every call to a service must settle within the `timeout` its `service.toml` names, in milliseconds, or 200, counted from when its service is ready to take it, so a service starting never eats into it. One deadline covers a call and the calls it makes. When it passes, the call rejects with a `504 TIMED_OUT`: a write still waiting its turn is refused when it comes, one running rolls back as though its handler threw, and whatever the handler answers later is dropped. A watch whose GET times out gets the error, and runs again after the next commit that writes what it read.
- **`watch(key, fn)`** runs a GET, then again after each commit that wrote a table it read. `fn` gets each result, or `undefined` and the error when the GET fails. It returns a function that stops the watch.
- **Lifecycle:** before a service applies anything on this device it initializes, in three steps that each end: its package (compiling it; a package this device does not have cannot be fetched without peers, so that step fails at once), its `setup`, and its health check, when its `service.toml` names one (`health = "/health"`: a GET it keeps trying until it answers without an error). Starting is bounded on its own: each space has the `startup_timeout` its `service.toml` names, 10000 milliseconds by default, to become ready. While it starts, its writes wait in its log and its GETs are held, their own deadlines not yet running. Then it is idle and answers them, or it stops with why: every call and watch on it gets a `503 SERVICE_STOPPED` carrying the cause, such as `PACKAGE_UNAVAILABLE` or `STARTUP_TIMEOUT` (with the last failure its health check met). Adding its package later initializes it again.
- **Run state:** `runState(service)` on a channel handle says where a service the channel names is in its lifecycle, or the channel itself with no name: `{state: 'initializing', services}`, `{state: 'idle'}`, `{state: 'applying', op}`, or `{state: 'stopped', error}`. `GET [@carbon]/status` reports the same `state`, and `error` once stopped, for a service and for each service of a channel. `watchRunState(service, fn)` calls `fn` with each change and returns a function that stops it. A service also stops on a system failure, such as a full disk, an I/O error or another process holding its file, until the problem is fixed by hand and `resume(service)` is called, for example from a button. The op that was applying applies again then, and `resume` resolves once the watches on the service have run again; a service stopped while initializing initializes again.
- **Handlers** receive `{path, input, env, call, callsync, effect}`, and `op` when the call writes. `env` holds the service's `sql` binding under the name its `service.toml` gives it, as a Worker's `env` does: `({env: {db}}) => db.all(...)` for `binding = "db"`. A write runs in one transaction, committed when the handler resolves and rolled back when it throws. Each call runs in an instance of its own, so a service keeps nothing in memory between calls; a handler that computes past its deadline is stopped there. `call(key, input)` to the same service makes a call inside it: it sees the handler's writes so far, and when it fails only its own writes roll back and the error rejects the `call`. `call` to another service the channel names is a message to it (see Across services): on the op's first run it is made, and what it answered is recorded with the op; every later run, on another device or after a rewind, gets the recorded answer instead. When this device holds no copy of the other service it can run, the call fails as unreachable. `callsync(key, input)` reaches another service consistently with the op instead (see Consistent across services). `effect(name, input)` makes any other effect the same way: `[effects.<name>] replicate = "always"` in `service.toml` makes it again on every run instead, never recorded. A GET reads committed state, never waits for a write, and cannot call one. `op` carries the author, device and time, with `id()` and `random()` derived from the op, the same on every replica and never the same twice within one op. Validating input is the service's own business: throw `validationError`.
- **Across services:** through `call`, only delivery is guaranteed between services. A call to another service posts an op there, which applies once, in that service's shared order, on every device that holds it. Its answer is what that service answered on this device at that moment, not a commitment like an HTTP 200: a late op from another device can still change the outcome there, and the other service never reports the change back, so a handler that needs the current answer asks again. Only a service's own tables change together: when a handler fails after another service answered, what that service did stays. Keep data that must change together, and the rules that hold across it, in one service, or reach the other service through `callsync`. When a step made through `call` in another service must be undone after a later failure, undo it as a saga does, with a compensating call the handler makes, or a later op. A write posted into a service whose op is waiting on the call, through calls or a `callsync` read, is refused at once with `508 LOOP_DETECTED`, since it would wait for itself.
- **Consistent across services:** `callsync(key, input)` keeps a call to another service the channel names consistent with the op making it, on every replica. A GET through `callsync` reads that service as of the op's place in the channel's shared order, whatever it has applied since on this device, and the op runs again when an op placed earlier there changes a row it read. It waits until that service has applied every op it holds before the op's place, within the op's deadline. Each statement of the GET must look up one row by its whole key: a search is refused with a `400`, the same on every replica, so search through `call`, then read the ids found through `callsync`. A write through `callsync` is bound to the op, and undone when the op is refused. From a GET, which has no place, `callsync` reads what is there now.
- **Builtin calls:** on `carbon`, `POST [@carbon]/identity` creates an identity, its first device, its own channel and its personal channel, and `GET [@carbon]/identities` lists them. On a channel, `GET [@carbon]/status` describes the channel, or one `service` it names, for the acting identity; `POST [@carbon]/channel` creates a channel nested in it and answers its ref, `POST [@carbon]/services` creates a service it names, and `GET [channel]/refs` and `GET [channel]/services` list them; on an identity, `GET [identity]/devices` lists its devices. A handle has no methods beyond these calls: reach what a call created through `ref`. Calls that need peers, such as `POST [channel]/members`, reject with `503 UNREACHABLE`. The whole interface is in [stories/services.md](../../docs/services.md).

CarbonDB binds no protocol. To serve a channel over HTTP, write an adapter that turns a request into `channel.call(key, input)` and a service error into a response.

## Building

The core is Rust, loaded through napi-rs. `npm run build` compiles it with `cargo`, copies the module beside `src/`, and copies the WIT world services build against into `wit/`. `npm test` first builds the test services under `fixtures/` into components.
