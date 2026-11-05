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
- **`carbon.services.add(name, impl)`** registers a service on every start. `impl` is the service's exported object: `migrations`, a list of SQL strings, and `calls`, keyed by service key, each with a `handler`. A device refuses a space naming a service it has not registered.
- **Handles** reach spaces, one per space: `carbon.ref('identity:alice').ref('personal:alice').ref('space:lobby')`. A handle acts as the identity its ref chain passed through.
- **`call(key, input)`** resolves with the handler's result once this device has applied it, or rejects with a service error from [`@tlsft/service-error`](https://www.npmjs.com/package/@tlsft/service-error): status, code, message and details intact.
- **`watch(key, fn)`** runs a GET, then again after each commit that wrote a table it read. `fn` gets each result, or `undefined` and the error when the GET fails. It returns a function that stops the watch.
- **Handlers** receive `{path, input, op, db}`. A write runs in one transaction, committed when the handler resolves and rolled back when it throws. `op` carries the author, device and time, with `id()` and `random()` derived from the op, so take ids, time and randomness from it, never from `Date.now()` or `Math.random()`. Validating input is the service's own business: throw `validationError`.
- **Builtin calls:** `GET [@carbon]/status`, `GET [space]/refs`, `GET [device]/refs`, and the helpers `createIdentity`, `resumeIdentity`, `listIdentities`, `createSpace`, `listSpaces` and `status`. Calls that need peers, such as `POST [@carbon]/members`, reject with `501 SINGLE_NODE`.

CarbonDB binds no protocol. To serve a space over HTTP, write an adapter that turns a request into `space.call(key, input)` and a service error into a response; `examples/chat-server` in this repository is one.

## Building

The core is Rust, loaded through napi-rs. `npm run build` compiles it with `cargo` and copies the module beside `src/`.
