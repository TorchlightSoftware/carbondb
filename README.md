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

Early design and prototyping. After evaluating existing platforms, p2panda (on iroh) is the base for a first command-line demo: create a room, invite with consent, post offline and merge, enroll a second device, remove a member, and talk through an always-on Pull-only relay.

## Repository

- [components/](components/README.md): one file per architectural component (identity, device constellation, contacts, private communication, blind relay nodes, state synchronization, storage, wallet, client platform, service calls), with what is agreed and what is open.
- [platforms/](platforms/README.md): evaluations of the candidate platforms and the feature matrix.
- [spikes/p2panda-demo/](spikes/p2panda-demo/README.md): the p2panda chat demo.
- [perspectives/](perspectives/): reviews and position notes from contributors.

Terminology follows the [Vision for Decentralized Web](https://github.com/bitmage/vision-for-decentralized-web) wiki.
