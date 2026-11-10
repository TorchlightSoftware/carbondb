# CarbonDB

A foundation in Rust for local-first applications that people own: identity, storage, communication and state synchronization in one core, with chat and a crypto wallet as the first use cases, and a device constellation so one person can use the same identity across many devices.

## Why

Decentralized programming is hard, and economic incentives keep leading us back to centralized services "for now". "for now" becomes "forever". The platforms that try to change this either do too little: leaving peer discovery, sync and device management for every developer to rebuild; or bundle so much runtime and domain language that they become difficult to adopt and apply to diverse use cases.

CarbonDB is an experiment in creating a general purpose framework for applications that prefer local-first state and personal data custody.

## Goals

- **Device constellation.** One identity across many devices, with Keybase's ergonomics: enroll a new device by scanning a QR code on one you already have, revoke a lost device, and recover with a paper key.
- **Chat.** End-to-end encrypted conversations between identities, where every member sees the same messages and conversations keep working while any one member's phone is offline.
- **Crypto wallet.** The core supplies identity, consent and communication; custody and transactions use the correct primitives for each currency rather than a ledger of our own.
- **Rust core, thin clients.** Identity, storage, communication and sync are written in Rust, with UI bindings for Flutter and Vue.js first. For the browser, the core runs as WebAssembly or behind a desktop shell.

## Principles

- **Local first.** Applications read and write local data; synchronization happens inside the core.
- **Consent.** Data is shared only with the people and devices a user chooses, and the user approves access, updates and transfers. Keys stay behind a boundary that asks before signing.
- **No owner, no single source of truth.** Shared data is the merge of signed logs from each writer, converging by the same rules on every device.
- **Blind infrastructure.** Always-on relays keep conversations available, but hold only ciphertext: they may store and serve data, never read or write it.
- **Data-centric apps.** An app is a layer for interpreting and changing data, not a silo that owns it. Personal data such as passwords and notes lives in the same core.
- **Portable services.** Primitives such as RPC and access control are designed so that existing business services can port to this architecture.

## Status

Early prototyping. p2panda (on iroh) is the base for replication. The single-node hello world runs: [chat] on the Rust core with its JS runtime, served over HTTP by an example app. Replicated mode, with p2panda spaces, is next. The design is in [stories/](stories/README.md).

`npm test` runs everything: the core's Rust tests, a native build, and the JS and end-to-end tests. It needs Rust and Node 22.

## Repository

- [perspectives/brandon/components/](perspectives/brandon/components/README.md): one file per architectural component (identity, device constellation, contacts, private communication, blind relay nodes, state synchronization, storage, wallet, client platform, service calls), with what is agreed and what is open.
- [perspectives/brandon/platforms/](perspectives/brandon/platforms/README.md): evaluations of the candidate platforms and the feature matrix.
- [stories/](stories/README.md): what a person can do with CarbonDB, and the agreed design underneath.
- [crates/core/](crates/core/): the Rust core: spaces on SQLite, the op log and results, applying each space's pending ops in turn, routing every call by service name, namespacing each service's tables, and `[@carbon]`, the part of the core that answers as a local service.
- [crates/service-bindings/](crates/service-bindings/): ServiceBindings, the toolkit a Rust service builds with. At runtime, the API footprint it exposes: its name, version, `init` and entry point, and the handler context each call receives; an opt-in helper that applies D1-style migrations; and the content hash every replica agrees on. Behind its `manifest` feature, a package's `service.toml` and its version, the hash of every file in the package, which the core reads for npm packages; at build time, behind its `build` feature, packaging a Rust service from them.
- [crates/services/](crates/services/): the builtin services `[space]`, `[identity]` and `[device]`, each a crate with its own `service.toml`, migrations and source. They link into the core natively for now, and will compile to WASM.
- [crates/node/](crates/node/): the core's Node bindings, through napi-rs.
- [packages/carbondb/](packages/carbondb/README.md): `@tlsft/carbondb`, the JS runtime apps use.
- [packages/services/chat/](packages/services/chat/): the [chat] service.
- [spikes/](spikes/): experiments: the p2panda chat demo, an async JS handler inside a Rust transaction, and SQLite sessions on tables without a primary key.
- [perspectives/](perspectives/): reviews and position notes from contributors.

Terminology follows the [Vision for Decentralized Web](https://github.com/bitmage/vision-for-decentralized-web) wiki.
