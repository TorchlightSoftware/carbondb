# nostr

Research date: 2026-10-26. See [README](README.md) for the vocabulary and the cross-platform matrix.

## Verdict for chat + wallet

nostr covers more of the chat and wallet surface than any other candidate today: relays store and forward ciphertext, Marmot brings forward-secret MLS groups, secp256k1 makes Bitcoin, Lightning and Cashu natural, and a Flutter app (0xchat) already ships all of that together. The architectural costs are a single permanent key that cannot rotate or be revoked, no User Keychain, no Device Constellation, and relays as a point of metadata visibility and censorship.

## Status

- **What it is:** a protocol, not one implementation. No token; development is funded by grants such as OpenSats. The NIPs repo is active (about 3.1k stars, commits daily). [NIPs](https://github.com/nostr-protocol/nips)
- **Dart libraries:**
  - NDK 0.10.1 (2026-10-02, MIT), with inbox/outbox (gossip) relay routing by default. [ndk](https://github.com/relaystr/ndk)
  - dart_nostr 10.0.1 (2026-04-23).
- **Marmot (MLS over nostr):** spec status "adopted". [Marmot](https://github.com/marmot-protocol/marmot) Least Authority finished its audit of whitenoise-rs on 2026-04-01. [audit](https://leastauthority.com/blog/audit-of-white-noise-whitenoise-rs/)
- **White Noise left Flutter:** its Flutter repo is archived (last Flutter release v2026.5.22, kept at tag `flutter-final`). It is being rebuilt natively in Kotlin and Swift on a Rust core called MDK. [README](https://github.com/marmot-protocol/whitenoise)

## Components

| Term | Provides? | How | Consequence for chat + wallet |
|---|---|---|---|
| Public & Secret Key | native | secp256k1 Schnorr keys, shown as npub/nsec | Same curve as Bitcoin |
| User Keychain | none | NIP-06 (seed-phrase derivation) is now "unrecommended: prefer a single nsec" ([index](https://github.com/nostr-protocol/nips)) | One key for every purpose |
| Self-Sovereign Identity | native | Generate a key; NIP-05 maps a name to it via DNS | Low friction; NIP-05 names depend on whoever runs the domain |
| Device Constellation | partial | NIP-46 remote signer. Marmot gives each device its own MLS member under one pubkey; full multi-device is still an idea ([identity](https://github.com/marmot-protocol/marmot/blob/master/foundation/identity.md)) | The nsec is copied or kept in a signer service. Paper backup is the nsec or a NIP-49 `ncryptsec` |
| Public Claims | native | Signed events; external identity proofs | Strong |
| Private Claims | partial | Gift-wrapped (NIP-59) events; MLS group state | Works |
| Contacts | native | Follow lists, NIP-05, npub QR codes | Ready to use |
| Peer Discovery | partial | Relay lists (outbox model); no peer to peer | Clients only ever talk to relays |
| Distributed Hash Table | none | Relays plus relay lists | Simple; relays are the point of control |
| Private Communication | native | DMs: NIP-17 + NIP-44 + NIP-59 hide metadata, but NIP-44 has "no forward secrecy" ([44](https://github.com/nostr-protocol/nips/blob/master/44.md)). Groups: Marmot/MLS gives forward secrecy and post-compromise security | Use Marmot for real security |
| Blind Relay Nodes | native | Relays store and forward ciphertext; NIP-42 AUTH limits who can read gift wraps ([17](https://github.com/nostr-protocol/nips/blob/master/17.md)) | Offline delivery comes free; relays still see IPs and timing |
| Encrypted Storage as a Service | partial | Relays and Blossom servers store ciphertext | Hosted by operators, not peers |
| State Synchronization | partial | Replaceable events; MLS epochs | No general CRDT layer |
| Public Ledger | none | No consensus mechanism | Money needs Lightning or Cashu |
| Currency / Wallet | via add-on | NIP-57 zaps (Lightning tips), NIP-47 Nostr Wallet Connect (draft), NIP-60/61 Cashu wallet and nutzaps (draft) | Bitcoin-only; community currencies only via your own Cashu mints *(inferred)* |
| Reputation / QoS | partial | NIP-13 proof of work, AUTH, paid and web-of-trust relays, NIP-85 Trusted Assertions | Spam is handled relay by relay |
| Public & Private Compute | partial | Logic runs in the client; relays are simple storage | Easy to run on mobile |

## Architectural decisions that matter most

1. **A single permanent key with no rotation.** No key-migration NIP exists in the index; the NIP-41 file returns 404; NIP-62 can only ask relays to delete a key's data. A 2026 guide says there is "no shipped mechanism" to rotate or revoke a key ([d-central](https://d-central.tech/nostr-key-management/), secondary, but consistent with the NIPs index). This hurts device recovery, and puts the wallet and the identity behind the same key if they share it.
2. **Dumb relays, smart clients.** Offline delivery and mobile support are easy. Relays see metadata and may censor.
3. **secp256k1.** Bitcoin, Lightning and Cashu tooling slots in naturally.
4. **MLS as a separate layer (Marmot).** Brings forward-secret groups, and a heavy Rust core to integrate.

## Existing apps

- **0xchat:** Flutter, MIT. v1.5.6 shipped 2026-09-30 ([releases](https://github.com/0xchat-app/0xchat-app-main/releases)). Built-in Cashu wallet ([1.2.6](https://nobsbitcoin.com/0xchat-v1-2-6)); MLS groups.
- **White Noise:** native rewrite in progress; last Flutter release v2026.5.22.

## Red flags and unknowns

- Why White Noise left Flutter.
- Whether MDK has any Dart binding; from Flutter it would likely mean flutter_rust_bridge or FFI *(inferred)*.
- Real-world adoption of NIP-17 and Nostr Wallet Connect was not verified.
