# A service's lifecycle

A service a channel names runs on every device that holds it. This page follows it on one device: how its package is registered and compiled, how each of its spaces starts before it applies anything, the states it moves through, and the deadline every call to it settles within. What a service is and how a channel names it are in [channels.md](channels.md); how its component runs, in [wasm-services.md](wasm-services.md).

## Registering a package

- **An app registers its packages on every start,** in `main()`: `await carbon.services.add('chat', chat)`. Nothing is stored in the database, and the handler code is never copied into it. A package is a WebAssembly component carrying its `service.toml`, given as a file URL, a path or its bytes.
- **It is registered at once,** under the name the app gives it, which overrides the name in its `service.toml`, so ops on it carry its version, the hash of its bytes. Its component starts compiling at the same time, off the core's thread ([wasm-services.md](wasm-services.md)). `services.add` resolves once it has compiled; awaiting it is optional, since calls made to its spaces meanwhile wait for them to start.
- **What is refused:** what is not a component, or a component without a `service.toml`, with a 400, as is one declaring a field CarbonDB does not know; a name already registered, CarbonDB's own (`channel`, `identity`) included, with a 409; and a component that does not compile, once it has tried, which leaves its name free and stops the spaces waiting on it with why.
- **The builtins,** `[channel]` and `[identity]`, are compiled as CarbonDB opens, which waits for them.

The core: `Carbon::add` and `Carbon::register` in `crates/core/src/carbon.rs`, and the registry in `crates/core/src/services/mod.rs`.

## Starting

Each service a channel names has a store of its own on each device that holds the channel: one SQLite file, holding its tables and results, beside the channel's one log. Before it applies anything here, it starts, in three steps:

1. **Its package.** It waits while this device compiles it. A package this device does not have cannot be fetched without peers, so single-node that step fails at once with `PACKAGE_UNAVAILABLE`; with p2panda it becomes a fetch ([p2panda-sync.md](../stories/p2panda-sync.md)).
2. **Its setup,** the service's `setup`, run on its `sql` binding in a transaction of its own, then the check that every table takes its keys from the op.
3. **Its health check,** when its `service.toml` names one: `health = "/health"`, a GET each of its spaces answers once set up. It passes when the GET answers without an error, and is tried again every 100 ms until it does. A service naming none is ready once set up.

Starting is bounded on its own: each space has the `startup_timeout` its `service.toml` names, 10000 milliseconds by default, from when it begins, to become ready. When it runs out, the space stops with `STARTUP_TIMEOUT`, carrying the last failure its health check met.

A space starts when this device opens it, when it is created, and again when a package it waited for is registered or a space stopped while starting is resumed. Adding a package later starts again the spaces that stopped for want of it. The core: `crates/core/src/channels/lifecycle.rs`.

## States

A space is in one state at a time:

| State | What it does |
|---|---|
| `initializing` | It is starting. Its writes wait in its log and its GETs are held. |
| `idle` | It is ready, with nothing to apply. |
| `applying` | It is applying one op; others wait their turn in its log, since a service writes one op at a time. |
| `stopped` | A start step failed, its startup ran out, or a system failure stopped it, such as a full disk, an I/O error or another process holding its file. Every call and watch on it gets a `503 SERVICE_STOPPED` carrying why, and nothing is logged or applied. |

Once a space is ready, the writes that waited apply in order, then the GETs that were held run. `resume(service)` on a channel handle resumes a stopped space once its cause is fixed: one never set up starts again; any other applies its pending ops again from the first, the op that was applying when it stopped included, and `resume` resolves once the watches on it have run again. An op logged without a result, as after a crash, applies when its space is next ready.

`runState(service)` on a channel handle reads a space's state, and `watchRunState(service, fn)` hears each change; `GET [@carbon]/status` reports the same `state`, and `error` once stopped, for a service and for each service of a channel ([services.md](services.md)).

## Deadlines

- **Every call settles within its service's `timeout`,** in milliseconds, 200 by default, or rejects with a `504 TIMED_OUT`. One deadline covers a call and the calls it makes inside it.
- **It counts the call's own time,** from when its space is ready to take it: at once when it already is, or when the space finishes starting. A slow start never eats into it; starting has its own bound.
- **Where the call is when it passes:** a write still waiting its turn is answered and then refused when its turn comes, with that 504 as its result, so it never applies after its caller was told it failed; a running write rolls back as though its handler threw; whatever the handler answers later is dropped. A watch whose GET times out gets the error, and runs again after the next commit that writes what it read.
- **A component that computes without yielding** is made to yield every 10 ms, so its deadline can pass and drop its instance ([wasm-services.md](wasm-services.md)).

The core: `crates/core/src/calls/deadline.rs`.
