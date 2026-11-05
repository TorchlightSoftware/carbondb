# Services ship as hashed packages that declare their bindings

Status: agreed — the builtins are packages with their own service.toml and migrations, linked natively; the WASM host and package hashes are not built

## Why

A replica must never run a call against a version of a service it doesn't have. Today nothing versions a service: builtins are Rust match arms with SQL in string constants, app services are JS objects, and an op records only a schema version, the service's number of migrations. Two devices can run different handler code under the same schema version and drift silently.

The fix is one package form for every replicated service, builtins included: a single artifact, hashed, whose hash every op records.

## The package

- A package is one artifact with a known entry point: a WebAssembly module (preferred) or an npm package (for code hard to convert).
- Its hash is its version. It covers the calls the service offers, its schema and its handlers. Implementations in other languages are simply other versions.
  - WASM: the hash of the module's bytes, computed when CarbonDB loads it.
  - npm: the integrity npm records in the lockfile, as npm gives it. A host may pass any non-empty string instead. The hash is required everywhere.
- CarbonDB is unopinionated about a package's insides. It reads only its `service.toml` and its entry point.
- The entry point receives every call for its service and routes method and resource itself. CarbonDB routes only to the package, by service name. For npm packages, the JS runtime's route matcher stays as the adapter from a `calls` object to that entry point.

## service.toml

CarbonDB's own manifest, baked into the package and covered by its hash: a service asking for different bindings is a different version. It holds:

- the service's name;
- `vars`: constants the handlers read;
- `sql` bindings: a binding name and its migrations. The count of a binding's migrations is its schema version.

The runtime refuses a `service.toml` with any field it does not know. There are no environments: each space and service is isolated already, so one config serves every device.

A generator writes `service.toml` from a `wrangler.toml` for services ported from Cloudflare Workers. It runs in one mode, such as production, and emits only that environment, without environment sections. `d1_databases` become `sql`, `vars` carry over, and every field it drops is reported as a warning. `r2_buckets` are left for [file bindings](backlog/file-bindings.md).

## Bindings

- A handler sees only bindings to storage that already exists, as a Worker sees `env.DB`. It never creates storage, generates keys or touches the system database.
- A space keeps every service's tables in its one database file, namespaced per service, so an op touching several services commits atomically: SQLite commits across attached files atomically per file only in WAL mode. For now a service declares at most one `sql` binding.
- A device sets up a space's bindings when it applies the op that creates the space or changes its services: it creates each binding's file and runs its migrations. This is internal to the core; nothing calls it directly. Every replica does it locally on receiving the op.
- A WASM module reaches its bindings through host functions linked into its instance: `run`, `get` and `all` on a named `sql` binding, plus `call` for calls on other services, such as `[space]`. The host resolves a binding name to that service's own file, so a module can never name another service's storage. npm packages receive the same shape as JS objects.

## The op

Every op carries the hash of the package that handled its call, and one migration count per `sql` binding. This replaces the single schema version.

## Builtins

- `[space]`, `[identity]` and `[device]` become WASM packages with their own `service.toml`. Their table definitions become migrations there; their queries stay inline in the handlers.
- They are embedded in the CarbonDB library as `.wasm` bytes, each keeping the hash of its own bytes. The CarbonDB version doesn't stand in for them, so devices on different CarbonDB releases still agree whenever a builtin is unchanged.
- Each builtin's `.wasm` is built once and committed, and the library embeds the committed file. A test fails when a builtin's source changed without a rebuild, since a different compiler may emit different bytes from the same source.
- `[@carbon]` is not a package. It never replicates, so no peer checks its version, and it drives the device's p2panda node, keys and setup. It stays host code in Rust.

## Registering

```js
carbon.services.add_npm('books', books, hash)
carbon.services.add_wasm('chat', './chat.wasm')
```

- CarbonDB never downloads or finds npm packages: the app installs them, imports them and registers them. Workspace-linked packages have no integrity and are not accommodated.
- CarbonDB offers a utility that looks up an installed package's integrity in the lockfile, for `add_npm`.
- Only WASM packages may later be installed automatically, by a manager, into a space.

## Build order

1. `service.toml`, table namespacing per service, and the package hash and migration counts in the op, with npm packages. The [chat] service becomes an npm package with a `service.toml`. The builtins already carry a `service.toml`, read by their build scripts.
2. The WASM host: wasmtime, the host functions, and `add_wasm`.
3. `[space]`, `[identity]` and `[device]` as committed WASM packages embedded in the library.
4. The `wrangler.toml` generator and the integrity lookup utility.

## Testing

- Unit tests in the core:
  - `service.toml` parsing, including refusing unknown fields;
  - two services' tables are namespaced apart in one file;
  - migrations are counted per binding;
  - the op envelope carries the package hash and the counts;
  - a builtin's committed `.wasm` matches its source.
- Runtime tests:
  - `add_npm` refuses a missing hash;
  - `add_wasm` hashes the module's bytes;
  - the integrity lookup reads a lockfile;
  - two services declaring the same table name don't collide in one space.
- Generator tests:
  - a `wrangler.toml` with several environments yields only the chosen one;
  - every dropped field is warned.
- End to end: the chat example runs on the packaged [chat] service, once as npm and once as WASM, with the e2e suite unchanged.

## Docs this changes

When this is built, update:
- [story 4](04-register-services.md): registration;
- [implementation.md](implementation.md):
  - Stack: handlers and route definitions;
  - The op: schema version;
  - Core and runtime: who owns routing and the builtins;
- the backlog story on [old ops](backlog/old-ops.md): what is hashed is now the package.
