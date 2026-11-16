# Services run as WebAssembly components

Every service a channel names runs as a WebAssembly component on the core's WASM host, whatever language it was written in. Only `[@carbon]`, the device's own service, is host code. This page explains what a service is, how it runs, what it can reach, and what to watch as the toolchains it depends on evolve.

## Why components

- **One boundary for every language.** A service sees the core only through one WIT world, `carbondb:service` ([crates/service-bindings/wit/service.wit](../crates/service-bindings/wit/service.wit)). The builtins are Rust, [chat] is JavaScript, and both are the same kind of package.
- **The host decides what a service reaches.** A component reaches the outside world only through the imports its host links. Replicas re-run every op's handler, so anything a handler reaches outside its own tables must be made once and replayed after, or be the same on every replica. On the WASM host that is enforced, not asked for.
- **A package is its bytes.** Its version is the hash of the component, which carries its `service.toml`, so every replica that runs an op runs exactly the code it was made for.

## A package

A package is one component that carries its `service.toml` in a custom section, `carbondb:service.toml`. Packaging appends the section to a component a toolchain built ([package.rs](../crates/service-bindings/src/package.rs)); registering reads it back, refuses a component without one, and versions the package by the hash of its bytes.

- **Rust:** a service crate written against ServiceBindings exports itself with `carbondb_service_bindings::serve!(PACKAGE)`, built for `wasm32-wasip2` with the `guest` feature. `carbondb-package` embeds its `service.toml`. [scripts/build-components](../scripts/build-components) does both for the builtins, `[channel]` and `[identity]`, and for the core's test fixture `[probe]`.
- **JavaScript:** `carbondb-build <service.js> <service.toml> <out.wasm>`, or `buildService` from `carbondb/build`, bundles the service with its dependencies into one module with Rolldown, compiles it with jco's QuickJS backend (`jco componentize --backend quickjs`, through componentize-qjs), and embeds its `service.toml`. A `.sql` file imports as its text, so a service keeps D1-style migration files. The service's module exports `calls` and an optional `setup`; `carbondb/service` turns them into the world's exports inside the component.

The builtins are committed under [crates/core/builtins/](../crates/core/builtins/) and embedded in the core, so devices on different CarbonDB releases agree whenever a builtin is unchanged. Beside each `.wasm` is a `.source` file, the hash of the sources it was built from: the service's crate and ServiceBindings' `src` and `wit`. A core test fails while a committed component is older than its sources; run `scripts/build-components` and commit what it writes.

## The world

What a service may ask of the core, all for the call it is handling:

| Import | What it does |
|---|---|
| `run(sql, params)` | a statement on its `sql` binding, inside the op's transaction; never in a GET |
| `all(sql, params)` | the rows a query answers, from the op's transaction or the GET's snapshot |
| `draw()` | the next value drawn from the op: ids and randomness, the same on every replica, never twice within one op, even across calls inside it |
| `call(key, input)` | a call by service key: inside the op to the same service, a message (the `call` effect) to another the channel names |
| `effect(name, input)` | an effect, made on the op's first run and replayed after (see [replication.md](replication.md)) |
| `log(level, message)` | a message for whoever runs the device, never part of the op |

What the core calls: `setup`, which sets up its schema inside a transaction on every open and runs to completion before it returns; and `handle`, async, once per call, with the request as JSON (`{method, resource, input, actor, op, binding}`), answering the call's value or a service error. Values cross as JSON text, failures as a service error's JSON.

## How a call runs

- **One instance per call.** Each call, a call inside an op included, runs in a fresh instance, as a task on the core's thread. A service keeps nothing in its instance between calls: its state is its `sql` binding, as a Worker's is its bindings.
- **On the core's thread.** Wasmtime asks that what an instance holds be thread-safe, and the core is single-threaded. An instance holds only the id of the call it runs for, and its imports reach that call through the table of calls running on the core's thread.
- **Deadlines.** An instance that computes without yielding is made to yield every 10 ms (wasmtime's epoch interruption), so its call's deadline can pass. When a call expires, the core settles it with a timeout and drops the task running its instance.
- **Traps.** A trap fails the call with a 500 and refuses its op; the next call runs in a fresh instance.
- **Compiling, off the core's thread.** Before a component runs, wasmtime compiles it to this machine's code ([crates/core/src/wasm/compile.rs](../crates/core/src/wasm/compile.rs)). Two threads only compile, each compile spread across cores, so the core keeps answering while one runs. The builtins compile as CarbonDB opens, which waits for them. An app's package starts compiling as it is registered, and is registered at once, so ops on it carry its version: its spaces initialize meanwhile, holding their calls (see the lifecycle in [crates/core/src/lifecycle.rs](../crates/core/src/lifecycle.rs)). A package that fails to compile is not registered after all: the app hears why, and the spaces waiting on it stop with it.
- **Compiled once, kept on disk.** One engine serves the whole process, and each version compiles once in it. Wasmtime's cache keeps compiled code in `compiled` beside the system database (configurable in `carbondb.toml`), so a restart loads it instead of compiling. Compiled code holds only for the wasmtime and settings that made it, which the cache's keys cover. The first CarbonDB a process opens chooses the directory for every one it opens after. Loading compiled code trusts that directory: whoever can write it can run code in the process, as whoever can write the data directory can already read the device's keys.

## What a service reaches outside the world

WASI is linked closed: no environment, arguments, files, sockets or standard input, and standard output goes nowhere. A call that writes sees the op's time on every clock and draws its randomness from the op's hash, so every replica's run sees the same; a GET, which no replica repeats, sees this device's clocks and randomness. Inside a JavaScript component, `Date.now()` reads that clock, and `console` logs through `log`.

### HTTP, through the core

A service reaches the outside world over `wasi:http`, 0.2's outgoing handler or 0.3's async client, the only interfaces beyond the core's own that the world imports (their WIT is in ServiceBindings' `wit/deps/`); both reach the host through the same hook. Every outgoing request is the core's `fetch` effect ([crates/core/src/ops/fetch.rs](../crates/core/src/ops/fetch.rs)), whatever library sent it: in a write it is made once, on the op's first run, and every replay gets the recorded response; in a GET it is made every time. The host takes a request's slot in the op's effects when the instance sends it, so requests sent together are recorded in the order sent, whatever order they answer in ([crates/core/src/wasm/http.rs](../crates/core/src/wasm/http.rs)).

- **Rust** and any other language with a `wasi:http` library send requests as they would anywhere. Rust's HTTP libraries mostly use WASI 0.2, and Rust has no stable `wasm32-wasip3` target yet, so ServiceBindings' world shares the `wasip2` crate's bindings.
- **JavaScript** calls the global `fetch`, which `carbondb/service` installs over WASI 0.3's async client ([packages/carbondb/src/service/fetch.js](../packages/carbondb/src/service/fetch.js)), with small `Headers` and `Response`, since QuickJS has no web APIs. A request in flight leaves the handler's other work running, as in any JavaScript runtime. A failed request rejects with a `TypeError`, as `fetch` does.
- **A response is read whole before it is answered**, since a write records all of it. A write cannot stream a response; a GET, never recorded, could stream one, but does not yet.
- **HTTPS** goes through wasmtime's TLS, which verifies a server against the Mozilla root certificates bundled into the library (`webpki-roots`), not the system's: a server whose certificate chains only to a private or corporate root is refused. A core test that reaches a server on the internet is opt-in: `cargo test -p carbondb -- --ignored`.
- **Recorded failures replay too.** A request that fails is recorded failing, and every replay meets the same failure; a replay sending a request its first run never made finds it unreachable.

### `Math.random` in JavaScript

`carbondb/service` replaces `Math.random` at the start of every call ([packages/carbondb/src/service/index.js:88](../packages/carbondb/src/service/index.js)): in a write it draws from the op, as `op.random()` does, so every replica draws the same; in a GET it draws from a generator seeded by this device's clock. Every JavaScript service gets this, not only ours, since `carbondb-build` serves every module through `carbondb/service`.

A value a module draws while it loads, such as a library seeding itself at import, is drawn once, at build time: componentize-qjs evaluates the module inside the Wizer snapshot it builds the component from, so that value is fixed per build and the same in every instance. Nothing at run time can reach it.

JavaScript builds are not deterministic, and cannot be made so without a change upstream: building `packages/carbondb/fixtures/counter` twice gave two components of the same size, 1,543,487 bytes, differing in 160 bytes in small clusters, consistent with clock or random state read while the snapshot was taken. componentize-qjs runs Wizer itself and offers no way to fix the clock or randomness it sees. So builds keep real entropy. Correctness does not rest on it: a package's version is the hash of the bytes shipped, and devices run those bytes, never rebuild them.

## The core's thread

The core runs on a thread of its own (`carbondb::Runtime`), driven by a single-threaded tokio runtime: its timers are tokio's, and every component's call is a local task there. Node's binding is a thin wrapper that sends the core work and hands JS its answers, which cross back through one courier, in the order the core gave them. The core's own tests drive their tasks by hand instead, as they drive its timers.

## What to watch

The toolchains under JavaScript services are young. Watch:

- **componentize-qjs.** Its async imports still have open issues (#101, #112), and it has no `fetch` or `console` of its own (#104); `carbondb/service` supplies both. jco 1.42 ships componentize-qjs 0.5; 0.6 is out. It is young, with one main author, who also contributes to jco and StarlingMonkey.
- **jco's bundling.** `jco componentize --bundle` swaps Node globals such as `console` for adapters that import `jco:node/*` interfaces outside our world, and writes those interfaces into the WIT directory it was given. `carbondb/build` bundles with Rolldown itself instead and hands jco one module.
- **ComponentizeJS gaining async imports.** StarlingMonkey's components imported only synchronous functions when this was built; once they are async, it becomes an alternative engine, faster but about 8 MB per component.
- **WASI 0.3 in wasmtime.** WASI 0.3.0 was ratified in June 2026, and JavaScript's `fetch` uses its HTTP client. wasmtime-wasi-http 50 still calls its 0.3 module experimental, and ships the WIT of 0.3.0 while 0.3.2 is out; a guest built against a newer 0.3 may need a newer wasmtime.
- **Rust on WASI 0.3.** Once Rust has a stable `wasm32-wasip3` target and its HTTP libraries move to 0.3, Rust services can follow; the host links both.
- **WASI 0.3 across toolchains.** Wasmtime 50 runs component-model async by default; the guests here use async exports and imports on WASI 0.2 interfaces. Moving WASI itself to 0.3 changes the clock, random and HTTP interfaces the host links.
- **Module size.** A JavaScript component is about 1.5 MB, mostly QuickJS; a Rust builtin about 300 KB.
- **The name `init`.** componentize-qjs reserves `init` for its build-time snapshot, so a service's schema export is `setup`.
