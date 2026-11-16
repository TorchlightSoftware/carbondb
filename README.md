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

The core API currently runs in single-node mode (no network). Each service runs in a channel, with its own SQLite file and op log, and services call each other by message. Rewind and replay are built and tested on a single device: ops received from other devices apply in one shared order, a late op rewinds only what it must, and what a service does outside its own tables (an effect) is made once and replayed after ([docs/replication.md](docs/replication.md)). Every service runs as a WebAssembly component on the core's WASM host, whatever language it was written in ([docs/wasm-services.md](docs/wasm-services.md)). Next step is to carry ops between devices with p2panda (on iroh). The design is in [stories/](stories/README.md), and the channel model's interface in [stories/channels/services.md](stories/channels/services.md).

This should be considered a first draft that shows you what the API surface looks like, and what it looks like to build an app on top of CarbonDB.

**What a service looks like**:

- a package: a WebAssembly component carrying its `service.toml`, built from Rust or JavaScript
- a `setup` that sets up its tables
- handlers keyed by [service key](https://www.npmjs.com/package/@tlsft/service-key), meant to be close to REST but decoupled from the transport layer
- handlers perform SQLite queries and return JSON

Example service call from [packages/services/chat/src/index.js](packages/services/chat/src/index.js#L43-L51):

```js
'POST [chat]/messages': {
  tags: ['append'],
  handler: ({input, op, env: {db}}) => {
    const {text} = parse(MessageInput, input)
    const id = op.id()
    db.run('INSERT INTO message (id, author, text, sent_at) VALUES (?, ?, ?, ?)', [id, op.author, text, op.time])
    return {id}
  }
},
```

**What app code looks like:** load CarbonDB, register the services, and reach a channel through refs, creating it on the first start. From [packages/services/chat/test/chat.test.js](packages/services/chat/test/chat.test.js#L20-L35), where `component` is [chat]'s package:

```js
async function start(config) {
  const carbon = await configure(config)
  carbon.services.add('chat', component)
  const identities = await carbon.call('GET [@carbon]/identities')
  const known = identities.some((identity) => identity.slug === 'app')
  if (known) {
    return carbon.ref('identity:app').ref('personal:app').ref('channel:lobby')
  } else {
    await carbon.call('POST [@carbon]/identity', {slug: 'app', device: 'server'})
    const personal = carbon.ref('identity:app').ref('personal:app')
    await personal.call('POST [channel]/channel', {name: 'Lobby'})
    const lobby = personal.ref('channel:lobby')
    await lobby.call('POST [channel]/services', {name: 'chat'})
    return lobby
  }
}
```

The app then calls the channel's service, `lobby.call('POST [chat]/messages', {text: 'hello'})`, and watches it, `lobby.watch('GET [chat]/messages', render)`.

**Built on single-node:**
 - create cryptographic identities KeyBase style, with multiple devices attached to an identity
 - secret keys stay on device and are never transmitted over the network
 - an identity's own channel, shared only by its devices, and its "personal channel" for apps and data
 - channels for organizations, or friends & family, nested for teams and departments
 - each service in its own SQLite file with its op log: oplog sync, rewind, playback (CRDT semantics)
 - services call each other by message; effects are made once and replayed after
 - every service a WebAssembly component on the core's own thread: the builtins in Rust, [chat] in JavaScript.
 - `fetch` for services over `wasi:http`, each request recorded as an effect.

The interface is in [packages/carbondb/README.md](packages/carbondb/README.md).

**Next:**
    - replication: p2panda carries each service's ops between members' devices, which adding devices and members needs.

`npm test` runs everything: the Rust tests, a native build, the JS services built into components, and the JS and end-to-end tests. It needs Rust and Node 22. After changing a builtin service or ServiceBindings, `scripts/build-components` rebuilds the committed components; it needs the `wasm32-wasip2` target (`rustup target add wasm32-wasip2`).

## Repository

- [perspectives/brandon/components/](perspectives/brandon/components/README.md): one file per architectural component (identity, device constellation, contacts, private communication, blind relay nodes, state synchronization, storage, wallet, client platform, service calls), with what is agreed and what is open.
- [perspectives/brandon/platforms/](perspectives/brandon/platforms/README.md): evaluations of the candidate platforms and the feature matrix.
- [stories/](stories/README.md): what a person can do with CarbonDB, and the agreed design underneath.
- [crates/core/](crates/core/): the `carbondb` crate, the Rust core: one SQLite file per service with its op log and results, applying each one's pending ops in turn, resolving every call through its channel, effects made once and replayed, the WASM host every service runs on, the runtime that runs the core on a thread of its own, and `[@carbon]`, the part of the core that answers as a local service. The builtins it embeds are committed in `builtins/`.
- [crates/service-bindings/](crates/service-bindings/): ServiceBindings, the toolkit a Rust service builds with: the `carbondb:service` WIT world every service exports; the API footprint a service exposes, its `setup`, entry point and the handler context each call receives; with its `guest` feature, exporting a Rust service as a component; an opt-in helper that applies D1-style migrations; the content hash every replica agrees on; and `carbondb-package`, which embeds a `service.toml` in a component.
- [crates/services/](crates/services/): the builtin services `[channel]` and `[identity]`, each a crate with its own `service.toml`, migrations and source, written against ServiceBindings alone and built as components by [scripts/build-components](scripts/build-components); and `[probe]`, a component the core's tests run.
- [crates/node/](crates/node/): the core's Node bindings, through napi-rs.
- [packages/carbondb/](packages/carbondb/README.md): `carbondb`, the JS runtime apps use.
- [packages/services/chat/](packages/services/chat/): `@tlsft/carbondb-svc-chat`, the [chat] service.
- [spikes/](spikes/): experiments: the p2panda chat demo, an async JS handler inside a Rust transaction, and SQLite sessions on tables without a primary key.
- [perspectives/](perspectives/): reviews and position notes from contributors.

Terminology follows the [Vision for Decentralized Web](https://github.com/bitmage/vision-for-decentralized-web) wiki.
