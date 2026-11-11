# carbondb-service-bindings

ServiceBindings: the toolkit a Rust [CarbonDB](https://github.com/TorchlightSoftware/carbondb) service builds with.

At runtime, it is the API footprint a service exposes: its name and version, an `init` that sets up its schema, and one entry point that receives each call as a handler context. A service knows nothing of the runtime that runs it. `migrations` is an opt-in helper that applies D1-style migrations from `init`, and `hash` is the content hash every replica agrees on.

At build time, behind the `build` feature, it packages a service: list this crate under `[build-dependencies]` with `features = ["build"]`, and call `carbondb_service_bindings::build::package(".")` from `build.rs` for a crate that is one service, with its `service.toml` beside its `Cargo.toml`. The service's version is the hash of every file in its directory.

Licensed under either of MIT or Apache-2.0, at your option.
