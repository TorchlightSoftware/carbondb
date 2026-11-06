# Holochain

Research date: 2026-10-26. See [README](README.md) for the vocabulary and the cross-platform matrix.

## Verdict for chat + wallet

Holochain is the closest match to the wiki's Public Ledger vision: each app network validates its own data with its own rules, with no global chain, which suits community currencies. For a mobile chat + wallet it is a poor fit today: no key rotation or device management since Deepkey was removed, no Dart client, no mobile template, double spends only detected after the fact, and minor releases that break the network.

## Status

- **Releases:** stable is 0.7.1 (2026-10-14). 0.7.0 came out 2026-07-30 and 0.6.3 on 2026-07-15. 0.8.0-dev builds ship weekly. [releases](https://github.com/holochain/holochain/releases)
- **Activity:** daily commits on the core repo and on kitsune2.
- **Licence and language:** core, hdk and hdi are Rust under CAL-1.0; kitsune2 is Apache-2.0.
- **Mobile:** 0.7 added an interpreted WASM backend (wasmi) to satisfy App Store rules. Only a diagnostic app has run on an iPhone, and there is "no ready-made mobile template" ([Dev Pulse 156](https://blog.holochain.org/dev-pulse-156-holochain-0-7-0-faster-simpler-apple-scented/)). Android is "experimental" through the Tauri-based p2p Shipyard ([FAQ](https://darksoil.studio/p2p-shipyard/documentation/faqs.html)).
- **Flutter/Dart:** no official Dart client; official clients are JavaScript and Rust ([tools](https://holochain.org/tools-and-libraries)). Nothing found on pub.dev.
- **Funding:** Holo's HOT is an ERC-20, due to migrate to HoloFuel. A migration test with about 80 testers is done ([May 2026](https://blog.holochain.org/ongoing-delivery-demonstrating-whats-next/)). A 2026-11-01 start date appears only in secondary sources *(unverified)*.
- **Organisation:** Holo shifted focus to Unyt and Edge Nodes, and its executive director changed ([Oct 2025](https://blog.holochain.org/finding-our-edge-a-strategic-update/)).

## Components

| Term | Provides? | How | Consequence for chat + wallet |
|---|---|---|---|
| Public & Secret Key | native | Ed25519 signing, X25519 + XSalsa20-Poly1305 encryption, Blake2b hashing, held in the Lair keystore ([crypto](https://developer.holochain.org/build/cryptography-functions/)) | Solid primitives; not secp256k1, so not native to Bitcoin or Ethereum |
| User Keychain | partial | Lair holds many keys; the agent key is per app *(inferred)* | Any keychain model is built on top |
| Self-Sovereign Identity | native | The agent creates its own keypair | No registry, but no human-readable name either |
| Device Constellation | none | DPKI/Deepkey removed in 0.6.0-dev.0 ([CHANGELOG](https://github.com/holochain/holochain/blob/develop/CHANGELOG.md)); Deepkey repo idle since 2025-04. 0.8-dev adds restoring a chain from the DHT, but only with the same key | Multi-device, revocation and recovery are ours to design |
| Public Claims | native | Signed source-chain entries that peers validate | Good fit for verifiable claims |
| Private Claims | partial | Private entries; membrane-gated networks *(inferred)* | Membership is per network, not per record |
| Contacts | none | Public keys exchanged out of band *(inferred)* | Discovery is ours to build |
| Peer Discovery | native | Bootstrap server plus iroh relay, then Kitsune2 gossip | The bootstrap service is a soft central dependency |
| Distributed Hash Table | native | Sharded storage and validation, one DHT per app network ([validation](https://developer.holochain.org/concepts/7_validation/)) | Needs peers online; mobile peers churn |
| Private Communication | partial | Encrypted transport; Relay claims end-to-end encryption but documents no protocol ([Relay](https://blog.holochain.org/happs-spotlight-relay/)) | No standard forward-secret group protocol; build or port one |
| Blind Relay Nodes | partial | iroh relays forward encrypted QUIC traffic | No store-and-forward; offline delivery relies on DHT peers *(inferred)* |
| Encrypted Storage as a Service | partial | Edge Nodes run always-on nodes ([2025 review](https://holo.host/blog/2025-year-in-review-the-year-we-built-the-edge-XqpCNKmMRVh/)) | Self-run or community-run; no paid market yet |
| State Synchronization | partial | Append-only source chains plus an eventually consistent DHT | CRDTs needed on top for chat history |
| Public Ledger | native, scoped | Each network's validators run app-defined deterministic rules; warrants block bad agents ([DP153](https://blog.holochain.org/dev-pulse-153-holochain-0-6-released-with-immune-system/)) | No global consensus; double spends detected, not prevented ([countersigning](https://developer.holochain.org/concepts/10_countersigning/)) |
| Currency / Wallet | via ecosystem | Unyt mutual credit, Circulo app, HoloFuel; countersigning is "unstable" and off by default | Community currencies fit well; tooling is young |
| Reputation / QoS | partial | Membrane proofs gate joining; warrants block bad actors. Membrane checks happen after joining, not at the handshake | Sybil resistance is designed per app; hosting incentives not live |
| Public & Private Compute | native | WASM runs on each user's device; Edge Nodes can host it | A Rust conductor must be embedded in the app |

## Architectural decisions that matter most

1. **Agent-centric validation scoped to each network.** Private community currency networks without a global chain, which suits a wallet. Cost: no finality, and double spends are only detected after the fact.
2. **The full conductor runs on the device.** No servers, but a heavy Rust runtime on mobile and a Flutter bridge to build ourselves *(inferred)*.
3. **Removing DPKI.** Simpler core, but no path to add devices or revoke keys.
4. **Network-breaking minor releases.** 0.5 moved to Kitsune2; 0.7 changed the wire protocol and database ("a clean break"). Deployed apps must upgrade all users together.

## Existing apps

- **Chat:** Relay (Android, 2024); Volla Messages (beta, reviews call it basic, [heise](https://heise.de/-10352930)); Moss, a suite of 17 tools including chat.
- **Wallet:** Unyt with the Circulo app. HoloFuel has not launched.
- **Acorn:** current state not verified.

## Red flags and unknowns

- No mobile template exists.
- p2p Shipyard required a licence as of 2024.
- Relationship between Volla Messages and Relay not confirmed.
- HoloFuel launch date unconfirmed.
- CAL-1.0 licence implications for our app not yet assessed.
