# @tlsft/carbondb

Write a service the way you write an ordinary backend service, with named calls, SQL tables and handlers, and CarbonDB keeps its state on the devices of the people using it.

This release runs **single-node**: one device, no peers. The app interface is the one replicated mode will keep, so switching later is a config change, not a rewrite.

## Use

```js
import {configure} from '@tlsft/carbondb'
import * as chat from './services/chat.js'

const carbon = await configure('./carbondb.toml')
carbon.services.add('chat', chat)

const {personal} = await carbon.createIdentity({slug: 'alice', device: 'laptop'})
const room = await personal.createSpace({name: 'Lobby', services: ['chat']})

await room.call('POST [chat]/messages', {text: 'hello'})
const stop = room.watch('GET [chat]/messages', (messages) => render(messages))
```

```toml
# carbondb.toml
system = "data/system.db"   # space files sit beside it

[sync]
mode = "none"               # single-node
```

## The interface

- **`configure(path)`** loads CarbonDB from its TOML file and returns `carbon`, the system handle.
- **`carbon.services.add(name, impl)`** registers a service on every start. `impl` is the service's exported module: `manifest`, where its package's `service.toml` is (`new URL('./service.toml', import.meta.url)`); `calls`, keyed by service key, each with a `handler`; and an optional `init(db)`, which sets up the service's schema on every open, inside a transaction, and finishes before it returns. CarbonDB reads the `service.toml`, and refuses with a 400 a service that exports none, or whose `service.toml` it cannot read or holds a field it does not know. The service runs under `name`, which overrides the name in its `service.toml`. Its version is the hash of every file in its package's directory, leaving out `node_modules`. A space naming a service this device has not registered opens and stays initializing: a GET to that service rejects with a 404, and every op on the space is logged and waits until the service is registered. An op made for a service this device has not registered is refused with a 404 when its turn comes. A name already registered, CarbonDB's own included, is refused with a 409, and a name holding a slash with a 400.
- **Schema:** a service names its tables plainly; CarbonDB keeps them as `service/db/name` in the space's one file, and refuses anything outside them. A row's key must come from the op that creates it, never from SQLite: after `init`, a table whose key is SQLite's rowid (`INTEGER PRIMARY KEY`, with or without `AUTOINCREMENT`) or that has no primary key is refused, with an error saying what to change. Use `op.id()`, or `id INTEGER PRIMARY KEY DEFAULT (op_id())` in a `WITHOUT ROWID` table for an integer key. `@tlsft/carbondb/migrations` applies a directory of D1-style migrations from `init`: `init: (db) => applyMigrations(db, readMigrations(url))`.
- **Handles** reach spaces, one per space: `carbon.ref('identity:alice').ref('personal:alice').ref('space:lobby')`. A handle acts as the identity its ref chain passed through. `ref()` answers at once, read from the space's refs as committed, and throws a 404 for a ref that is not there.
- **`call(key, input)`** resolves with the handler's result: a GET at once, and a write once this device has applied it and every watch its commit touched, or that was running when it landed, has run again. It rejects with a service error from [`@tlsft/service-error`](https://www.npmjs.com/package/@tlsft/service-error): status, code, message and details intact.
- **Timeout:** every call to a service must settle within the `timeout` its `service.toml` names, in milliseconds, or 200. One deadline covers a call and the calls it makes. When it passes, the call rejects with a `504 TIMED_OUT`, a write rolls back as though its handler threw, and whatever the handler answers later is dropped. A watch whose GET times out gets the error, and runs again after the next commit that writes what it read.
- **`watch(key, fn)`** runs a GET, then again after each commit that wrote a table it read. `fn` gets each result, or `undefined` and the error when the GET fails. It returns a function that stops the watch.
- **Run state:** `runState()` on a space handle says where the space is in applying its ops: `{state: 'initializing', services}` while a service the space names is not set up here, `{state: 'idle'}`, `{state: 'applying', op}`, or `{state: 'stopped', error}`. `watchRunState(fn)` calls `fn` with each change and returns a function that stops it. A space stops on a system failure, such as a full disk, an I/O error or another process holding its file: every call and watch on it then gets a `503 SPACE_STOPPED` carrying the failure, and nothing is logged or applied, until the problem is fixed by hand and `resume()` is called, for example from a button. The op that was applying applies again then.
- **Handlers** receive `{path, input, env, call}`, and `op` when the call writes. `env` holds the service's `sql` binding under the name its `service.toml` gives it, as a Worker's `env` does: `({env: {db}}) => db.all(...)` for `binding = "db"`. A write runs in one transaction, committed when the handler resolves and rolled back when it throws. `call(key, input)` makes a call inside it: it sees the handler's writes so far, and when it fails only its own writes roll back and the error rejects the `call`. A GET reads committed state, never waits for a write, and cannot call one. `op` carries the author, device and time, with `id()` and `random()` derived from the op, so take ids, time and randomness from it, never from `Date.now()` or `Math.random()`. Validating input is the service's own business: throw `validationError`.
- **Builtin calls:** `GET [@carbon]/status`, `GET [space]/refs`, `GET [device]/refs`, and the helpers `createIdentity`, `resumeIdentity`, `listIdentities`, `createSpace`, `listSpaces` and `status`. Calls that need peers, such as `POST [@carbon]/members`, reject with `501 SINGLE_NODE`.

CarbonDB binds no protocol. To serve a space over HTTP, write an adapter that turns a request into `space.call(key, input)` and a service error into a response.

## Building

The core is Rust, loaded through napi-rs. `npm run build` compiles it with `cargo` and copies the module beside `src/`.
