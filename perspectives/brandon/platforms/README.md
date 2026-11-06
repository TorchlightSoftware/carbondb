# Decentralized chat + crypto wallet: platform evaluation

Research date: 2026-10-26. Facts move fast in this space; check the dated status sections in each platform file before relying on them. Claims marked *(inferred)* were reasoned rather than read in a source; claims marked *(unverified)* came from secondary sources only.

## Target use case

A decentralized chat app with a built-in crypto wallet, probably with a Flutter client. Candidates evaluated: [Freenet](freenet.md), [Autonomi (formerly MAIDSafe / Safe Network)](autonomi.md), [Holochain](holochain.md), [KeyOxide](keyoxide.md), [Ockam](ockam.md), [nostr](nostr.md), [iroh and p2panda](iroh-p2panda.md).

## Conclusions

### No candidate covers chat plus wallet; the stack will be a composition

Every platform leaves at least one critical component missing or half-built. The real decision is which network anchors identity, which carries messages, which settles money, and what we build ourselves.

### Device Constellation is unsolved everywhere, so we build it

Not one candidate ships "one identity across many devices, add and revoke devices, recover from loss with a paper key". Holochain removed DPKI/Deepkey in 0.6. nostr has one permanent key and no rotation. KeyOxide ASP has no rotation or revocation. Freenet lists delegate sync as open. Autonomi says it has no recovery mechanism. Ockam has rotation and revocation but no multi-device enrollment or paper-key recovery. p2panda encrypts for multi-device groups but documents no device enrollment or recovery.

### Ockam's identity model is the best design to borrow for an identity root

The identifier is derived from the first signed key change, a signed change history rotates the primary key and revokes purpose keys, and credentials carry attributes checked by policy (ABAC). The project itself appears abandoned, so borrow the design, not the code.

### nostr has the broadest chat and wallet coverage today, at the cost of a permanent key and relay-visible metadata

Relays give offline store-and-forward for free; Marmot (MLS over nostr) gives forward-secret groups; secp256k1 is Bitcoin's curve, so Lightning and Cashu wallets slot in; 0xchat is an existing Flutter app that already ships MLS groups and a Cashu wallet. Missing: key rotation, User Keychain, Device Constellation, any ledger of its own.

### Holochain is the only candidate matching the wiki's "scoped ledger" vision, but it is a poor fit for mobile today

Each app network validates its own data with its own rules, which suits community currencies (Unyt mutual credit). But double spends are only detected after the fact, there is no device key management, no Dart client, no mobile template, and minor releases break the network.

### Freenet cannot hold a currency by design

Its merge-not-consensus model is excellent for chat state and the whitepaper itself says it cannot prevent double spending. Data that nobody requests can vanish from every host.

### Autonomi is a storage network; its chat lives on a separate young network

Pay-once permanent encrypted storage is its strength (attachments, backups). Messaging moved to x0x, which is pre-release and largely one author. Every write needs ANT plus ETH gas on Arbitrum.

### KeyOxide is a Public Claims add-on, not a network

Useful for verifiably linking an identity to existing accounts. Fragile maintenance and no key rotation in ASP.

### iroh and p2panda are the strongest Rust base for transport and sync, but bring no wallet or offline delivery

iroh (stable 1.x, MIT/Apache) dials Ed25519 keys rather than addresses, with hole-punching, relays and discovery we can host ourselves. p2panda (pre-1.0, MIT/Apache) adds append-only log sync, group membership as converging signed operations, and custom group encryption. Missing: Public Ledger, wallet, Contacts, Public Claims, and store-and-forward for offline phones.

### A Flutter client narrows the field to nostr, KeyOxide, iroh and (with a daemon) Autonomi

The strongest group-crypto cores (MDK for Marmot, OpenMLS) and p2panda are Rust, so Flutter means a Rust bridge (flutter_rust_bridge or FFI) regardless; iroh alone has a community Flutter plugin. White Noise abandoned its Flutter app for native Kotlin/Swift on its Rust core; the reason is unknown.

## Vocabulary

Component names come from the [Vision for Decentralized Web](https://github.com/bitmage/vision-for-decentralized-web) wiki. The reading used in this evaluation:

| Wiki term | Meaning here |
|---|---|
| Public & Secret Key | Asymmetric key primitives and the algorithms used |
| User Keychain | How a user holds many keys (per app, per purpose) |
| Self-Sovereign Identity (SSID) | Identity the user creates with no gatekeeper |
| Device Constellation | One identity across many devices: add, revoke, recover after loss, paper backup key |
| Public Claims | Signed statements anyone can verify, e.g. linking an identity to other accounts |
| Private Claims | Signed records shared only with members |
| Contacts | The address book: how you find and add one specific person |
| Peer Discovery | Finding nodes and peers without a central host (DHT, bootstrap list, gossip) |
| Distributed Hash Table | Whether one is used, and for what |
| Private Communication | End-to-end encrypted 1:1 and group messaging, forward secrecy, metadata privacy |
| Blind Relay Nodes | Relays that route or store-and-forward without seeing content; offline delivery; anonymization |
| Private Peers / Encrypted Storage as a Service | Encrypted data stored on other people's machines |
| State Synchronization | CRDTs or other merge models for shared state (chat history, multi-device sync) |
| Public Ledger | Consensus or validation for public data; whose agreement makes data valid (global vs scoped) |
| Cryptocurrency / Community Currency / Crypto Wallet | Native token or currency, scoped community currencies, wallet integration |
| Reputation / Quality of Service | Spam and Sybil resistance; incentives for relay and storage operators |
| Public & Private Compute | Where application logic runs: user device, network nodes, hosted servers |

## Feature matrix

● native · ◐ partial or via add-on · ○ none. All seven provide solid Public & Secret Key primitives, so that row is omitted.

| Wiki component | Freenet | Autonomi | Holochain | KeyOxide | Ockam | nostr | iroh + p2panda |
|---|---|---|---|---|---|---|---|
| User Keychain | ● delegates | ● HKDF child keys | ◐ Lair | ◐ profiles only | ● purpose keys | ○ single nsec | ◐ member key bundles |
| Self-Sovereign Identity | ◐ key per room | ● | ● | ● | ● | ● | ● |
| Device Constellation | ○ | ◐ x0x machine keys | ○ Deepkey removed | ○ | ◐ rotation only | ◐ NIP-46 signer | ◐ multi-device groups |
| Public Claims | ◐ Ghost Keys | ○ | ● | ● core purpose | ◐ credentials | ● | ○ |
| Private Claims | ◐ via contracts | ○ | ◐ | ○ | ● | ◐ gift wrap / MLS | ◐ signed membership ops |
| Contacts | ○ invite links | ◐ four-word addresses | ○ | ◐ lookup only | ○ | ● follows, NIP-05 | ○ tickets / QR |
| Peer Discovery | ● ring routing | ● Kademlia | ● Kitsune2 gossip | ○ | ○ | ◐ relay lists | ● DNS, Pkarr, mDNS |
| Distributed Hash Table | ● | ● | ● | ○ | ○ | ○ | ◐ Pkarr records |
| Private Communication | ◐ no forward secrecy | ◐ MLS on x0x, pre-release | ◐ undocumented | ○ | ◐ live 1:1 only | ● Marmot / MLS | ◐ custom group crypto, unaudited |
| Blind Relay Nodes | ◐ state persists | ◐ undocumented offline | ◐ live only | ○ | ◐ no store-and-forward | ● native | ◐ live relays only |
| Encrypted Storage as a Service | ◐ demand-based | ● pay once, permanent | ◐ Edge Nodes | ○ | ○ | ◐ Blossom, relays | ◐ blob transfer |
| State Synchronization | ● core feature | ◐ x0x CRDTs | ◐ source chains | ○ | ○ | ◐ replaceable events | ● append-only logs |
| Public Ledger | ○ by design | ◐ Arbitrum | ● scoped per network | ○ | ○ | ○ | ○ |
| Crypto Wallet / currency | ○ | ◐ ERC-20 | ◐ Unyt mutual credit | ○ | ○ | ◐ Lightning, Cashu | ○ |
| Reputation / QoS | ◐ rate limits | ● node incentives | ◐ membranes, warrants | ◐ weak signal | ○ | ◐ PoW, paid relays | ○ |
| Public & Private Compute | ● WASM contracts | ○ client only | ● WASM on device | — | ◐ nodes | ◐ client | ◐ client |
| Reachable from Flutter | ○ | ◐ via daemon | ○ | ● aspe-dart | ○ | ● NDK, dart_nostr | ◐ community iroh_flutter |
| Project health | alpha, active | production, rebuilt 2026 | stable, network-breaking releases | volunteer-run | dormant | active protocol | iroh stable 1.x; p2panda pre-1.0, active |

## Related

Each architectural component, with what is agreed and what is still open, has its own file in [components/](../components/README.md).
