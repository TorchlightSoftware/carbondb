# Ockam

Research date: 2026-10-26. See [README](README.md) for the vocabulary and the cross-platform matrix.

## Verdict for chat + wallet

Treat Ockam as a reference design, not a dependency. Development has effectively stopped and the team has moved on, its hosted relays appear gone, and its mobile and FFI layers were removed. But its identity model is the best of the six for an identity root: an identifier derived from the first signed key change, a signed change history that rotates the primary key and revokes purpose keys, and attribute credentials checked by policy (ABAC). Those ideas are worth reimplementing.

## Status

- **Releases:** last release v0.157.0 on 2025-06-30 ([GitHub API](https://api.github.com/repos/build-trust/ockam/releases)); last crates.io publish 0.150.0 on 2025-03-20.
- **Commit activity:** 100+ commits in June 2025, 17 in July 2025, 2 in September, 1 in November; 14 in January 2026, all by one person and mostly "zone dev" / `AiPlatformApi` gateway work; nothing since 2026-01-04.
- **Licence:** Apache-2.0. The LICENSE file dates from 2018, and every checked version of the `ockam` and `ockam_command` crates back to 0.0.0 (2021) is Apache-2.0 ([README](https://github.com/build-trust/ockam)). No licence change found. The hosted Orchestrator had separate terms of service, not read.
- **Language:** Rust, plus an Elixir implementation.
- **Mobile and FFI:** `ockam-ffi` was deleted in 2023-09; the Swift app (`implementations/swift`) and `ockam_app_lib` were removed on 2025-06-01. No Dart bindings; Flutter would need a hand-built `flutter_rust_bridge` binding over the Rust crates *(inferred)*.
- **Company direction:**
  - `ockam.io` redirects to `docs.ockam.io`; Orchestrator and account hosts did not respond.
  - The team moved to **Autonomy**, a hosted Python platform for AI agents, in the same GitHub org and active to 2026-03 ([repo](https://github.com/build-trust/autonomy)). It "leverages the open-source Ockam secure connectivity toolkit under the covers" ([Intellyx, 2025-10](https://intellyx.com/2025/10/10/autonomy-cloud-based-ai-agent-creation-and-deployment-platform-with-vertical-scalability/)).
  - An aggregator says Ockam "dissolved in July 2026", citing the company's LinkedIn page ([YesPress](https://yespress.io/ockam.md)) *(unverified)*.

## Components

| Term | Provides? | How | Consequence for chat + wallet |
|---|---|---|---|
| Public & Secret Key | native | Ed25519 (preferred) or P-256 identity keys; X25519 channel keys; AES-GCM/SHA256 or ChaChaPoly/BLAKE2s ([identities](https://docs.ockam.io/documentation/protocols/identities.md), [channels](https://docs.ockam.io/documentation/protocols/secure-channels.md)) | Solid primitives, independently reviewed by Trail of Bits |
| User Keychain | partial | Vaults (local or AWS KMS) plus "purpose key" attestations | A good model for per-purpose keys |
| Self-Sovereign Identity | native | Identifier = first 20 bytes of the SHA256 of the first signed key change | Self-created; in practice trust comes from an Authority |
| Device Constellation | partial | A signed change history rotates the primary key and can revoke all purpose keys | Has rotation; adding devices under one identity and paper-key recovery are not specified *(inferred)* |
| Public Claims | partial | Credentials: signed attribute statements any identity can issue | Built for service authorization, not public account linking |
| Private Claims | native | Credentials are presented only to the other party during the encrypted handshake | Suits group-membership tokens |
| Contacts | none | — | — |
| Peer Discovery | none | Explicit routes to nodes; relays are named at a known node | Needs a known rendezvous host |
| Distributed Hash Table | none | — | — |
| Private Communication | partial | Noise XX, mutually authenticated, keys rotated every 32 messages for forward secrecy | Live 1:1 channels only; no groups, no offline asynchronous ratchet *(inferred)* |
| Blind Relay Nodes | partial | Relays forward encrypted traffic across NAT; Orchestrator hosted them ([relays](https://docs.ockam.io/documentation/command/advanced-routing.md)) | Relays don't see content, but no store-and-forward and no anonymity *(inferred)*; hosted relays likely gone |
| Encrypted Storage as a Service | none | — | — |
| State Synchronization | none | — | — |
| Public Ledger | none | — | — |
| Currency / Wallet | none | — | — |
| Reputation / QoS | none | Access gated by the issuing Authority | Admission centralized per trust context |
| Public & Private Compute | — | Actor-style workers on nodes (device or server) | Logic runs wherever nodes run |

## Architectural decisions that matter most

1. **Application-layer routing independent of transport.** Works over TCP, UDP, BLE and WebSocket, useful for NAT traversal, but assumes peers are online: a poor fit for mobile chat.
2. **Trust anchored in an Authority issuing attribute credentials, checked by ABAC policy** ([access controls](https://docs.ockam.io/documentation/protocols/access-controls.md)). Suits scoped communities; the Authority is a gatekeeper, which cuts against self-sovereign identity.
3. **Noise XX secure channels.** Strong for live 1:1 sessions; no group protocol and no offline delivery.
4. **A hosted Orchestrator for relays and enrollment.** It was the easy path, and now appears gone.
5. **Removal of mobile and FFI layers.** Flutter integration would be entirely our own work.

## Ideas worth borrowing

- Identifier derived from the first signed key change, so the identity survives key rotation.
- Signed change history for rotation and revocation.
- Purpose keys attested by the identity key (a User Keychain model).
- Attribute credentials presented privately in the handshake and checked by ABAC policy; this resembles a chain of signed grants narrowing from a root key to devices.

## Existing chat and wallet apps

None. Use cases were enterprise data connectivity (Kafka, Postgres, AWS Marketplace listings) ([docs index](https://docs.ockam.io/llms.txt)).

## Red flags and unknowns

- The company has effectively stopped working on Ockam, possibly dissolved *(unverified)*.
- No release in about 16 months; no team behind the code; hosted service status unknown.
