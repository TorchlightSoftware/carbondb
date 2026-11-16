# carbondb-service-bindings

ServiceBindings: the toolkit a Rust [CarbonDB](https://github.com/TorchlightSoftware/carbondb) service builds with.

At runtime, it is the API footprint a service exposes: a `setup` that sets up its schema, and one entry point that receives each call as a handler context. A service knows nothing of the runtime that runs it. `migrations` is an opt-in helper that applies D1-style migrations from `setup`, and `hash` is the content hash every replica agrees on.

Every CarbonDB service runs as a WebAssembly component of the `carbondb:service` world, in `wit/service.wit`. A Rust service crate exports itself as one: build it for `wasm32-wasip2` with this crate's `guest` feature, and call `carbondb_service_bindings::serve!(PACKAGE)` with its `Package`. The world imports `wasi:http`, 0.2's outgoing handler and 0.3's client, whose WIT is in `wit/deps/`: a Rust service sends HTTP requests with any library on WASI 0.2, such as the `wasip2` crate the world's bindings share, and the core makes each one as its `fetch` effect, recorded in a write and replayed after.

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
carbondb-service-bindings = { version = "0.1", features = ["guest"] }
```

A package is that component carrying the service's `service.toml`. With the `manifest` feature, `carbondb-package <component.wasm> <service-dir> <out.wasm>` embeds the `service.toml` in the service's directory, and writes beside it the hash of the sources it was built from. The package's version is the hash of its bytes.

Licensed under either of MIT or Apache-2.0, at your option.
