# Services ship as hashed packages that declare their bindings

Status: agreed — every service but `[@carbon]` is a WebAssembly component carrying its service.toml, versioned by its bytes, on the WASM host ([docs/wasm-services.md](../../docs/wasm-services.md)): the builtins `[channel]` and `[identity]` committed and embedded, [chat] built from JavaScript; each service runs in a space of its own, one SQLite file, with no namespacing; a channel names the package each of its services runs. Open: the `wrangler.toml` generator, and how a package is identified across devices

## Why

A replica must never run a call against a version of a service it doesn't have. Before packages, nothing versioned a service: builtins were Rust match arms with SQL in string constants, app services were JS objects, and an op recorded only a schema version, the service's number of migrations. Two devices could run different handler code under the same schema version and drift silently.

The fix is one package form for every replicated service, builtins included: a single artifact, hashed, whose hash every op records.

## The package

- A package is one WebAssembly component of the `carbondb:service` world, carrying its `service.toml`, built from any language: the builtins from Rust, [chat] from JavaScript.
- Its hash is its version: the hash of the component's bytes, computed when CarbonDB registers it. It covers the calls the service offers, its schema, its handlers and its manifest. Implementations in other languages are simply other versions.
- CarbonDB is unopinionated about a package's insides. It reads only its `service.toml` and its entry point.
- The entry point receives every call for its service and routes method and resource itself. CarbonDB routes only to the package, by the name its channel gives the service. For JavaScript, `carbondb/service` routes a `calls` object inside the component.

## service.toml

CarbonDB's own manifest, baked into the package and covered by its hash: a service asking for different bindings is a different version. It holds:

- the service's canonical name. A host registers it under a name of its own, which overrides it on that device;
- `timeout`: the milliseconds every call to the service must settle within, 200 when it names none (see Timeouts and refusals in [implementation.md](implementation.md));
- `version`: the version the package declares, semver canonically. It is advisory: CarbonDB records it beside the hash and never trusts it, since only the hash says which code ran. Not built: the runtime refuses it as an unknown field for now;
- `vars`: constants the handlers read. Not built, and refused for now, like `version`;
- `sql` bindings: at most one binding name, since the service's tables, its op log and its results share one file. How a service shapes its storage, such as by migrations, is internal to it: its own `setup` applies and tracks them;
- `effects`: what the service does outside its own tables, by name, such as `[effects.fetch] replicate = "never"`. `never`, the default, makes the effect on an op's first run only and replays its recorded output; `always` makes it again on every run on every replica, never recorded. An effect left out takes the default. See [docs/replication.md](../../docs/replication.md#effects-are-made-once).

The runtime refuses a `service.toml` with any field or value it does not know, so a later setting is an addition. There are no environments: each service is isolated already, so one config serves every device.

A generator writes `service.toml` from a `wrangler.toml` for services ported from Cloudflare Workers. It runs in one mode, such as production, and emits only that environment, without environment sections. `d1_databases` become `sql`, `vars` carry over, and every field it drops is reported as a warning. `r2_buckets` are left for [file bindings](../backlog/file-bindings.md).

## Bindings

- A handler sees only bindings to storage that already exists, as a Worker sees `env.DB`. It never creates storage, generates keys or touches the system database. Creating a service's storage is an effect of `POST [channel]/services`.
- Each service runs in a space of its own: one SQLite file holding its tables beside CarbonDB's record of it (its op log and results, named `carbon_`), so an op and its transaction always cover the same data. Data that must change together in one op lives in one service; a call to another service is a message, and atomicity ends at the service.
- A device sets up a service when it creates the service's space, and again on every open: it calls the service's `setup` in a transaction of its own, outside any op, then checks its tables' keys. This is internal to the core; nothing calls it directly. A replica will do the same when it joins the service's space, which needs peers.
- A service's statements run as written, with no rewrite into a namespace. SQLite's authorizer refuses a statement that names CarbonDB's `carbon_` tables or reaches another file through ATTACH or a pragma, and records what each statement read.
- A component reaches its binding through the world's host imports: `run` and `all` on its `sql` binding, `draw` for the op's ids and randomness, `call` for calls on other services, such as `[channel]`, `effect` for its other effects, and `log`. The host resolves the binding to the service's own file, so a component can never name another service's storage.

## The op

Every op carries the hash of the package that handled its call. This replaces the schema version, the count of migrations.

## Builtins

- `[channel]` and `[identity]` are Rust crates under `crates/services/`, each with its own `service.toml` and migrations, built as components. `[device]` is merged into `[identity]`.
- They are embedded in the CarbonDB library as `.wasm` bytes, each versioned by the hash of its own bytes. The CarbonDB version doesn't stand in for them, so devices on different CarbonDB releases still agree whenever a builtin is unchanged.
- Each builtin's `.wasm` is built once by `scripts/build-components` and committed, and the library embeds the committed file. A test fails when a builtin's sources changed without a rebuild, since a different compiler may emit different bytes from the same source.
- `[@carbon]` is not a package. It runs only on this device, its ops go in the system log, which never replicates, and what it changes in channels it changes through their own calls, made as effects, so no peer checks its version. It drives the device's keys, setup and p2panda node, and stays host code in Rust.

## Registering

```js
import {component} from '@tlsft/carbondb-svc-chat'
carbon.services.add('chat', component)          // a package: a file URL, a path, or its bytes
```

- CarbonDB never downloads or finds packages: the app installs them, imports them and registers them. A package published on npm ships its built component.
- Only WASM packages may later be installed automatically into a channel's services: CarbonDB's core reads the channels its identity belongs to, joins their services and fetches their WASM.

## A channel names its services' packages

Status: built single-node for naming and resolving; fetching packages to join a channel needs peers.

A service name means something only inside a channel. A channel's `[channel]` keeps an entry for each service it runs: the name calls use there, the service's id, the package registered on this device that runs it, and its version. A call to `[name]` on a channel's handle looks the name up there, then the package in the device's registry. So CarbonDB's `[identity]` runs in `identity:alice` while an app's own role service, registered as `roles`, is named `[identity]` in `company:foo-corp`, and each service's tables stay apart, since each has its own file.

The entry is also what a peer needs to join a channel: which packages to register, or fetch, before each service's ops can apply. CarbonDB's core, in its init and maintenance loop, reads the channels its identity belongs to, joins their services and fetches their WASM, at the lower of the identity's grant and the device's policy. Not built: it needs peers and the WASM host.

A package's registered name is still device-wide: the core keeps one registry for the whole device, so a name CarbonDB already uses, such as `identity`, cannot be registered by an app. The channel gives the local `[name]` with `POST [channel]/services {name, package}`.

Open:

- How a package is identified across devices: a name unique across packages, as npm scopes give (`@tlsft/identity`, `@otto/identity`), or its version hash, which changes on every upgrade (see [moving a service to a new version](../backlog/upgrade-package-version.md)). Today the entry holds the name it is registered under on the device that created it, and its hash.

## Build order

1. `service.toml`, a file per service, and the package hash in the op. Built.
2. The WASM host: wasmtime, the world's host imports, closed WASI, `wasi:http` as the `fetch` effect, and registering a component. Built.
3. `[channel]` and `[identity]` as committed components embedded in the library. Built.
4. The `wrangler.toml` generator. Open: [registering a package by its name](../backlog/package-names.md).

## Testing

- Unit tests in the core:
  - `service.toml` parsing, including refusing unknown fields and values, and more than one `sql` binding;
  - a service's statements cannot reach `carbon_` tables or another file;
  - the op envelope carries the package hash;
  - a builtin's committed `.wasm` matches its sources (built);
  - a package is versioned by its bytes and refused without a service.toml (built).
- Runtime tests:
  - `add` refuses what is not a component carrying a service.toml (built);
  - two services declaring the same table name don't collide, each in its own file.
- Generator tests:
  - a `wrangler.toml` with several environments yields only the chosen one;
  - every dropped field is warned.
- End to end: the [chat] tests run on the packaged [chat] component (built).

## Docs this changes

Reflected in [story 4](04-register-services.md), [implementation.md](implementation.md), the [old ops](../backlog/old-ops.md) backlog story, and [docs/wasm-services.md](../../docs/wasm-services.md), which explains the WASM host.
