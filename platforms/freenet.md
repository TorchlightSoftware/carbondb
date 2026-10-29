# Freenet (new, Rust)

Research date: 2026-10-26. See [README](README.md) for the vocabulary and the cross-platform matrix.

**Disambiguation:** this is the new Freenet, the Rust rewrite formerly called Locutus. The original Java Freenet was renamed Hyphanet in March 2023; it is an anonymous store of immutable files and does not interoperate with the new Freenet. [F9]

## Verdict for chat + wallet

Freenet suits chat state well and wallets not at all. Its contracts define commutative merges, which is exactly what group chat and multi-device sync want, and River chat is live. But the October 2026 whitepaper says the merge model cannot prevent double spending, so a currency needs a different consensus layer. Data persists only while someone wants it, there is no network-layer anonymity, no mobile node and no Dart SDK.

## Status

| Item | Value |
|---|---|
| Maturity | Alpha. The quickstart warns that versions change often and old ones stop working [F3]. Network announced as live in March 2026 [F13]. |
| Latest release | v0.2.150, 2026-10-26; releases every few days [F1] |
| Licence and language | freenet-core AGPL-3.0, stdlib LGPL, River LGPL-3.0. All Rust; contracts and delegates are WebAssembly [F4][F5] |
| Network size | 396 active peers in a 2026-05-25 snapshot [F2] |
| Client SDKs | Rust and TypeScript only; apps talk to a local node over WebSocket [F11] |
| Mobile and Flutter | No official mobile node, no Dart SDK. River runs in a mobile browser against a hosted node; try.freenet.org runs on Freenet's own server "for now" [F10]. A Flutter app would talk to a node over WebSocket *(inferred)* |
| Funding | 501(c)(3) nonprofit, grants and donations [F12] |
| Team | Small; two people account for about 4,000 of the core commits [F14] |

## Components

| Term | Provides? | How | Consequence for chat + wallet |
|---|---|---|---|
| Public & Secret Key | native | Transport uses X25519 + AEAD; River uses elliptic-curve signing and ECIES (X25519 + AES-GCM); Mail uses post-quantum crypto [F2][F5][F10] | Standard primitives, but each app picks its own |
| User Keychain | native | Delegates hold keys on the device; "the private key never crosses the boundary" [F6] | A good home for wallet keys, but inside a Freenet node |
| Self-Sovereign Identity | partial | River uses a key per room; no global identity [F5] | Cross-app identity is ours to build |
| Device Constellation | none yet | Delegate sync across devices is "Open"; today a manual encrypted bundle export and import [F2] | Adding, revoking and recovery all ours |
| Public Claims | partial | Ghost Keys: certificates from a donation, blind-signed by Freenet's server [F7] | That server is the issuer, so it acts as gatekeeper |
| Private Claims | via app | Contract validity rules check signed records [F2] | Doable inside private-room contracts |
| Contacts | none | Invite links into rooms only [F5] | No way to find a person; out-of-band exchange needed |
| Peer Discovery | native | Gateways only for first connection, then small-world ring routing [F2] | Little dependence on bootstrap servers |
| Distributed Hash Table | native, DHT-like | Each contract's ring location comes from the hash of its code and parameters; replicated on nearby peers [F2] | Room state is found by contract key |
| Private Communication | partial | Private rooms: AES-256-GCM, key rotated weekly or on a ban; no per-message forward secrecy; metadata visible [F5] | Weaker than Signal or MLS; group size and timing leak |
| Blind Relay Nodes | partial | Hosting peers store encrypted state; no anonymity claimed [F9] | Offline delivery works because room state persists *(inferred)*; no onion routing |
| Encrypted Storage as a Service | partial | Data persists only while wanted; cold data is evicted and "can disappear from every host" [F2] | Old history and inactive inboxes need someone re-uploading them |
| State Synchronization | native, core feature | Each contract defines an idempotent commutative merge, plus summary/delta sync [F2] | A great fit for chat history |
| Public Ledger | none, by design | "Without global consensus"; consistency is per contract [F2] | No global ordering |
| Currency / Wallet | none | Double-spend prevention is impossible because "merge is total"; Harvest marketplace in early design [F2][F10] | Wallet needs an outside chain or separate consensus layer |
| Reputation / QoS | partial | Per-peer rate limits; antiflood tokens paid with donations; Sybil resistance "Open" [F2][F8] | No incentives for peer operators |
| Public & Private Compute | native | Contracts run as WASM on hosting peers; delegates on the user's device [F2] | Shared logic limited to deterministic validation and merge |

## Architectural decisions that matter most

1. **Merge instead of consensus.** Suits group chat and multi-device sync; cannot express a token balance, so the wallet lives elsewhere [F2].
2. **Data kept only while there is demand.** Active rooms stay available; inactive inboxes and identity records can be lost, bad for reliable offline delivery [F2].
3. **No network-layer anonymity.** Simpler and faster than old Freenet; metadata privacy must be built on top [F9][F5].
4. **Delegates as an on-device secret vault.** A clean place for keys; cross-device sync not built yet [F2][F6].
5. **Peer location derived from IP prefix.** A known Sybil and eclipse weakness, which also undermines reputation contracts [F2].

## Existing apps

- **River** (group chat): live; private rooms "still experimental" [F10].
- **Ghost Keys** (identity): deployed, but its key vault "can currently lose keys" [F7].
- **Mail:** pre-alpha, deployed. **Delta** (publishing): live. **Raven** (social feed): prototype. **Harvest** (marketplace): early design [F10].
- **Wallets:** none.

## Red flags and unknowns

- Ghost Key payments go through Stripe; critics say payment timing could link a donor to their key [F7].
- Transport docs describe RSA while the whitepaper says X25519; the docs are out of date [F15][F2].
- Mobile-node feasibility and battery use untested.
- Whether the AGPL core matters if our app only talks to a node over WebSocket has not been assessed.

## Sources

- [F1] https://github.com/freenet/freenet-core/releases
- [F2] https://freenet.org/pdf/freenet-whitepaper.pdf (dated 2026-10-03)
- [F3] https://freenet.org/quickstart
- [F4] https://raw.githubusercontent.com/freenet/freenet-core/main/LICENSE.md
- [F5] https://github.com/freenet/river
- [F6] https://freenet.org/build/manual/components/delegates
- [F7] https://freenet.org/ghostkey/ ; https://freenet.org/news/introducing-ghost-keys ; https://tbot.substack.com/p/freenet-one-big-privacy-problem
- [F8] https://freenet.org/build/manual/examples/antiflood-tokens
- [F9] https://freenet.org/about/faq/
- [F10] https://freenet.org/presentations/2026-07-17-apps-without-platforms/
- [F11] https://freenet.org/build/manual/
- [F12] https://freenet.org/
- [F13] https://tech.slashdot.org/story/26/03/15/0512246/
- [F14] https://api.github.com/repos/freenet/freenet-core/contributors
- [F15] https://freenet.org/build/manual/architecture/transport
