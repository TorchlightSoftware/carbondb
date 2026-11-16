# Components compile on a pool of their own

Status: agreed — its own commit, after the WASM host is committed

## Why

Before a component runs, wasmtime compiles it to this machine's code: about 2 s for a JavaScript service, about 0.2 s for a Rust builtin. Today that happens on the core's thread, inside `register`, so while it compiles no call, timer or watch on the device moves, and Node waits in `register`.

## Approach

- **A pool dedicated to compiling.** The core's runtime owns a small pool of threads that only compile components. Compiled components cross back to the core's thread by message; the core's thread never compiles.
- **Builtins compile on boot.** Every device uses `[channel]` and `[identity]`, so they start compiling as CarbonDB opens, on the pool.
- **Other services compile lazily, on the pool.** A service compiles the first time a space needs it. Every call that needs a component still compiling waits for that one compile, and each version compiles once per process.
- **Everything else keeps moving.** Calls on services already compiled answer while another compiles.
- **Compiled code is kept on disk.** Compiled components are cached in a directory beside the system database, keyed by package version and wasmtime version, since compiled code holds only for the wasmtime and CPU that made it. A restart loads them instead of compiling; a miss compiles on the pool and is written to the cache.

## Tests

- A call on one service answers while another service's component is still compiling.
- After open, a call on a builtin compiles nothing more.
- Two calls needing the same uncompiled version cause one compile.
- A second open with the same packages compiles nothing; a cache entry from another wasmtime version is not used.

## Open

- Whether `register` still refuses a component that does not compile, which compiling lazily would otherwise find only at first use.
- How many threads the pool has.
