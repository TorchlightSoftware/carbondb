# iroh and p2panda

Research date: 2026-10-26. See [README](README.md) for the vocabulary and the cross-platform matrix.

Evaluated together because p2panda is built on iroh: iroh is the transport, p2panda the local-first data, group and encryption layer above it. Either can be used without the other's higher layers.

## Verdict for chat + wallet

The strongest Rust base of the candidates for transport, discovery, sync and group encryption, and the best match for "networking and persistence in Rust". iroh is stable and heavily used; p2panda is active and well funded, but pre-1.0 with APIs that will break. Neither has anything to say about money, contacts, public claims or identity recovery, and neither stores messages for offline phones: delivery to a phone that is asleep needs an always-on peer of our own to sync through *(inferred)*. Use iroh as a dependency; treat p2panda as either a dependency we track through breaking changes, or as the reference design for our own sync and group layer.

## Status

### iroh

- **Releases:** 1.0.0 published to crates.io on 2026-06-15; current 1.4.1 on 2026-10-22 ([GitHub releases](https://api.github.com/repos/n0-computer/iroh/releases), [crates.io](https://crates.io/crates/iroh)). 1.0 promises wire-protocol and API stability ([write-up](https://pinggy.io/amp/blog/iroh_1_0_dial_keys_not_ips/)) *(unverified)*.
- **Licence:** MIT OR Apache-2.0 on crates.io.
- **Maker:** n0 (number 0), a company; repo pushed daily.
- **Self-hosting:** `iroh-relay` and `iroh-dns-server` are published at 1.4.1, so relays and discovery servers can be run by us rather than n0.
- **Bindings:** `iroh-ffi` (Apache-2.0) covers Python, Swift, Kotlin and Node for the stabilised 1.0 surface; `iroh-blobs`, `iroh-docs` and `iroh-gossip` are out of scope there because they are not yet 1.0 ([iroh-ffi README](https://github.com/n0-computer/iroh-ffi)). Community packages `iroh_flutter` and `iroh_quic` 1.3.0 (Apache-2.0, published 2026-10-26, about 800 downloads a month) cover Android, iOS, macOS, Linux and Windows ([pub.dev](https://pub.dev/packages/iroh_flutter)).
- **RPC:** `irpc` (Apache-2.0, active) is n0's streaming RPC crate for iroh ([repo](https://github.com/n0-computer/irpc)).

### p2panda

- **Releases:** 0.7.1 on 2026-08-21; 0.6.0 in May 2026 ([GitHub releases](https://api.github.com/repos/p2panda/p2panda/releases)). The README says APIs "are not yet considered stable for production use" until 1.0 ([README](https://github.com/p2panda/p2panda)).
- **Licence:** MIT OR Apache-2.0.
- **Maker:** a small collective, grant-funded by NLnet / NGI for group encryption and capabilities ([NLnet](https://nlnet.nl/project/P2Panda-groups/)); repo active to 2026-10-23.
- **iroh dependency:** `p2panda-net` 0.7.1 depends on `iroh` ^1.0.3, `iroh-gossip` ^0.101 (not 1.0) and `iroh-mdns-address-lookup` ([crates.io](https://crates.io/crates/p2panda-net)).
- **Bindings:** `p2panda-ffi` (UniFFI: Node, Python, Go) is experimental, last pushed 2026-07-29; no Dart.
- **Docs:** docs.rs reports `p2panda-spaces`, `p2panda-auth` and `p2panda-encryption` at 42–48% documented.

## Components

| Term | Provides? | How | Consequence for chat + wallet |
|---|---|---|---|
| Public & Secret Key | native | Ed25519 node and author keys; X25519, HPKE and XChaCha20-Poly1305 in p2panda-encryption ([docs](https://docs.rs/p2panda-encryption)) | Same curve as KeyOxide ASP; not Bitcoin's secp256k1 |
| User Keychain | partial | p2panda-spaces `Credentials` and member key bundles ([docs](https://docs.rs/p2panda-spaces)) | Keys per member; purpose keys not described |
| Self-Sovereign Identity | native | An identity is a key pair the user makes | No gatekeeper |
| Device Constellation | partial | p2panda-spaces is "data encryption for multi-device groups" | How devices join an identity, and recovery after loss, are not documented *(inferred)* |
| Public Claims | none | — | Pair with KeyOxide-style claims |
| Private Claims | partial | p2panda-auth membership operations are signed and shared within the group | Membership is data that travels with the group |
| Contacts | none | iroh tickets (key plus address) can be shared by QR or link | Address book is ours to build |
| Peer Discovery | native | iroh: signed records via DNS and Pkarr, local mDNS ([discovery](https://docs.iroh.computer/concepts/discovery)); p2panda: confidential topic discovery | Find a key's address without a central host, if we run our own servers |
| Distributed Hash Table | partial | Pkarr records can live on the Mainline DHT *(inferred)* | No DHT for application data |
| Private Communication | native | iroh: every connection end-to-end encrypted QUIC. p2panda: a group data scheme (shared key, rotated on removal, so removed members can't read new data) and a message scheme (Double-Ratchet-style, a key per message, forward secrecy) | Group encryption is custom, not MLS; no audit found |
| Blind Relay Nodes | partial | iroh relays forward encrypted traffic when hole-punching fails | Live only, no store-and-forward *(inferred)*; relays see IPs and timing |
| Encrypted Storage as a Service | partial | p2panda-blobs moves large files between peers | Transfer, not paid or persistent storage |
| State Synchronization | native | p2panda-sync over append-only logs; works with any CRDT; broadcast-only, so it also works over BLE, LoRa or USB | A merge model for chat history across devices |
| Public Ledger | none | — | — |
| Currency / Wallet | none | — | — |
| Reputation / QoS | none | — | Relay operators are not paid |
| Public & Private Compute | partial | Application logic runs on the client | Matches Rust core hosting JS services *(inferred)* |

## Architectural decisions that matter most

1. **Dial a key, not an address (iroh).** A peer is named by its Ed25519 public key; where it currently is gets looked up. This fits an identity-centric service key, where `{frank}` resolves to keys rather than hosts.
2. **Group membership is a signed operation graph that converges (p2panda-auth).** Four access levels, Pull < Read < Write < Manage, optionally narrowed by conditions such as data paths. Concurrent conflicting changes are resolved by a pluggable resolver; the default "strong removal" invalidates a removed manager's concurrent actions ([docs](https://docs.rs/p2panda-auth)). This is subjective authority built into the data: each replica decides validity from the same rules.
3. **Broadcast-only, offline-first (p2panda).** Data is append-only logs that any peer can relay, which suits the Mesh Network hope and chat history, but means there is no request/response primitive above iroh streams.
4. **Two encryption schemes (p2panda-encryption).** A data scheme where late joiners read history, and a message scheme with forward secrecy and the user-experience costs it brings. A chat app picks one per space.
5. **Stable transport, unstable layers.** iroh core is 1.x; `iroh-gossip`, `iroh-blobs`, `iroh-docs` and all of p2panda are pre-1.0.

## Ideas worth borrowing

- Membership as signed operations with a pluggable conflict resolver: a model for who has read and write access to a conversation.
- Access levels with conditions narrowing them, in the same shape as route-key globs narrowing a feature.
- Tickets (key plus relay address) as the QR payload for adding a contact.

## Existing chat and wallet apps

- **Delta Chat** uses iroh 0.35 and iroh-gossip in its core for realtime peer channels (`chatmail/core` [Cargo.toml](https://github.com/chatmail/core/blob/main/Cargo.toml)).
- **Reflection**, p2panda's collaborative local-first GTK text editor ([repo](https://github.com/p2panda/reflection)).
- No wallet found on either.

## Red flags and unknowns

- p2panda APIs will break before 1.0, and the higher layers are under half documented.
- p2panda's group encryption is a custom protocol with no audit found; the strongest audited alternative is MLS (OpenMLS).
- iroh's defaults use n0's relays and discovery servers; running our own is supported but is an operational cost.
- Keyhive (Ink & Switch, Apache-2.0, `keyhive-wasm` 0.4.0-alpha.1 on 2026-10-13) is the competing design for capability-based access control and encryption on shared documents ([lab notes](https://www.inkandswitch.com/keyhive/notebook/05/)); not evaluated here.
