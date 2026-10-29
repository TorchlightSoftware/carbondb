# Autonomi (formerly MAIDSafe / Safe Network)

Research date: 2026-10-26. See [README](README.md) for the vocabulary and the cross-platform matrix.

## Verdict for chat + wallet

Autonomi is a production network for paid, permanent, encrypted file storage, with EVM wallet support that comes almost for free. Chat is not built on the storage network: the 2.0 relaunch (around April 2026) reduced developer-facing storage to immutable chunks plus paid pointers, and messaging lives on x0x, a separate, pre-release gossip network written mostly by one person. Its most plausible role in our stack is attachments and backups.

## Status

| Item | Value |
|---|---|
| Maturity | Production storage network; ANT token launched February 2025 [A20][A22] |
| Autonomi 2.0 | A full replacement, not an upgrade; the team said data could not be moved "seamlessly" [A19]. Live around 2026-04-07 per a secondary source [A4]. The 1.0 repo was archived 2026-05-22 [A3] |
| Latest release | ant-node v0.23.1, 2026-10-22, roughly weekly [A1] |
| Licence and language | MIT/Apache-2.0, Rust [A2][A15] |
| Cryptography | ML-DSA-65 signatures and ML-KEM-768 key exchange, no classical fallback [A2] |
| Mobile and Flutter | Dart package `antd_client` talks REST/gRPC to a local `antd` daemon [A8]. Native Swift/Kotlin bindings via UniFFI [A7]. Roadmap lists "Mobile SDKs" as actively shipping [A11]. iOS and Android demos pay via WalletConnect [A15]. A practical Flutter route is platform channels or a Rust bridge *(inferred)* |
| Token | ANT, an ERC-20 on Arbitrum One; max supply 1.2 billion [A13][A14]. Node emissions paused 2026-01-20; whether they resumed is unverified [A4] |
| Team | MaidSafe plus the Autonomi Foundation in Geneva [A21]. ant-node has 8 contributors; x0x commits are almost all from founder David Irvine plus an AI coding agent [A16] |

## Components

| Term | Provides? | How | Consequence for chat + wallet |
|---|---|---|---|
| Public & Secret Key | native | ML-DSA-65 and ML-KEM-768; public keys 1,952 bytes, signatures 3,309 bytes [A9] | Quantum-resistant but large; handshakes about 7.5 KB |
| User Keychain | native | Unlimited child keys from one master secret via HKDF [A9] | Clean per-app or per-purpose keys; the wallet key is a separate EVM key [A5] |
| Self-Sovereign Identity | native | Identity is a key pair: "no usernames, passwords" [A9] | No gatekeeper |
| Device Constellation | partial (x0x) | Owner key, agent keys, a key pinned per machine [A12]; "no recovery mechanism" [A9] | Recovery and paper backup ours to build |
| Public Claims | none | No native statements linking an identity to other accounts | Build on signed chunks *(inferred)* |
| Private Claims | none | Not covered by the docs | Ours to build |
| Contacts | partial | Four-word addresses actively shipping [A11] | Possible base for person-to-person discovery |
| Peer Discovery | native | Kademlia with trust weighting; x0x relies on team-run bootstrap nodes in 6 cities [A2][A11] | x0x depends on those team-run nodes |
| Distributed Hash Table | native | Kademlia over XOR addresses; data replicated to close groups [A18] | Good for content addressing |
| Private Communication | via add-on (x0x) | MLS/TreeKEM groups with "forward-secrecy membership" [A11][A12] | Better forward secrecy than Freenet River; x0x is pre-release (v0.40/0.41) |
| Blind Relay Nodes | partial | QUIC NAT traversal and relays; offline store-and-forward in x0x undocumented [A12] | Offline delivery unproven |
| Encrypted Storage as a Service | native, core feature | Self-encryption, pay once, "no ongoing storage charges" [A5] | Good for media, backups, chat archives |
| State Synchronization | partial | 1.0 registers and scratchpads dropped from the 2.0 SDK; only paid pointers remain mutable [A7][A16]. x0x offers CRDT key-value stores and task lists [A12] | Live chat state lives on x0x, not on storage |
| Public Ledger | partial | Data needs no consensus; close groups verify chunks; payments settle on Arbitrum [A10][A5] | Data validity is global; money depends on Arbitrum |
| Currency / Wallet | via ecosystem | Any EVM wallet works, with external signers [A13][A5] | Easy wallet integration; community currencies would be ordinary ERC-20s on Arbitrum *(inferred)* |
| Reputation / QoS | native for nodes | EigenTrust scoring, ASN and geographic diversity, ANT rewards; "Trust Layer" is a proof of concept on Base testnet [A11][A2] | Real operator incentives; no user spam control |
| Public & Private Compute | none yet | Apps run on the client; "Compute Mesh" still being shaped [A11] | All logic on the device |

## Architectural decisions that matter most

1. **Immutable storage, paid once.** Good for permanent archives. Every pointer update costs roughly one chunk on chain, so per-message storage would be expensive *(inferred)* [A5][A7].
2. **Payments on Arbitrum.** Wallet support almost free; every write needs ANT plus ETH for gas, which is friction for chat users. A card payment gateway is still being shaped [A13][A11].
3. **Rebuilt from scratch in 2.0.** Modern crypto, but 1.0 apps and data types were left behind. The Friends chat app relied on scratchpads (*inferred* broken) [A19][A17].
4. **Messaging moved to x0x.** Proper MLS groups, but a second, young network mostly written by one person [A12][A16].
5. **Post-quantum only.** Future-proof, but large keys and signatures and no standard EVM or wallet tooling for these key types *(inferred)*.

## Existing apps

- **x0x** (agent-to-agent collaboration): running in production per the roadmap [A11].
- **Communitas** (Board, Chat, Docs, Calls on x0x): "actively shipping"; no repo found [A11].
- **Friends** (1.0-era chat over WebRTC with scratchpads for discovery): status on 2.0 unknown [A17].
- **Wallets:** generic EVM wallets (MetaMask, Ledger) plus WalletConnect mobile demos [A15].

## Red flags and unknowns

- Close-group size inconsistent: 5 in the docs [A18], K=20 in the ant-node README [A2].
- Legacy data handling contradictory: ant-node README says legacy data stays readable and free [A2]; the February update said no transfer [A19].
- Economic model reworked late; current emissions state unverified [A4].
- Forum posts after June 2026 could not be fetched.

## Sources

- [A1] https://github.com/WithAutonomi/ant-node/releases
- [A2] https://github.com/WithAutonomi/ant-node
- [A3] https://github.com/maidsafe/autonomi/releases
- [A4] https://coinmarketcap.com/currencies/autonomi/ (secondary)
- [A5] https://docs.autonomi.com/developers/core-concepts/payment-model.md
- [A6] https://docs.autonomi.com/developers/core-concepts/data-types.md
- [A7] https://github.com/WithAutonomi/ant-sdk
- [A8] https://docs.autonomi.com/developers/sdk/reference/language-bindings/dart.md
- [A9] https://docs.autonomi.com/how-it-works/encryption-and-authentication/multisig-credentials.md
- [A10] https://docs.autonomi.com/how-it-works/consensus-not-required.md
- [A11] https://withautonomi.github.io/autonomi-roadmap/content/roadmap.json
- [A12] https://github.com/saorsa-labs/x0x
- [A13] https://docs.autonomi.com/token/index.md
- [A14] https://docs.autonomi.com/how-it-works/network-economics/token-supply.md
- [A15] https://github.com/orgs/WithAutonomi/repositories
- [A16] https://github.com/WithAutonomi/ant-client ; GitHub contributor API for ant-node and x0x
- [A17] https://forum.autonomi.community/t/friends-the-messenger-youll-never-want-to-move-away-from/41688/221
- [A18] https://docs.autonomi.com/how-it-works/network-architecture/building-on-kademlia.md
- [A19] https://forum.autonomi.community/t/weekly-developer-update-february-19th-2026/42796 (read via search snippet)
- [A20] https://rexbytes.com/2026/05/06/autonomi-network-review/ (secondary)
- [A21] https://docs.autonomi.com/learn-more/autonomi-foundation.md
- [A22] https://docs.autonomi.com/token/converting-maidsafecoin/timeline.md
