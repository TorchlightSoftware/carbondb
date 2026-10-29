# Private Communication

Critical path: yes. Depends on: [Identity](identity.md), [Device Constellation](device-constellation.md), [Contacts](contacts.md).

## Context

- Wiki: Private Communication makes use of Device Constellation, User Keychain and Contacts.
- nostr NIP-17 + NIP-44 + NIP-59 gift-wrapped DMs hide metadata but NIP-44 has no forward secrecy.
- Marmot (MLS over nostr) has adopted status; Least Authority audited whitenoise-rs on 2026-04-01; its Rust core is MDK.
- p2panda group encryption offers post-compromise security and optional forward secrecy; it is a custom protocol, not MLS, and no audit was found.
- Keyhive (Ink & Switch, Rust, Apache-2.0, active) is an alternative design for capability-based access control and encryption of shared documents.
- Autonomi x0x has MLS/TreeKEM groups, but is pre-release (v0.40/0.41) and written mostly by one person.
- Freenet River private rooms use AES-256-GCM with the key rotated weekly or on a ban; no per-message forward secrecy; group size and timing leak.
- Holochain Relay claims end-to-end encryption but documents no protocol.
- Ockam: Noise XX for live 1:1 channels only; no groups and no offline ratchet.
- An earlier KeyOxide + Flutter session proposed OpenMLS from Flutter via flutter_rust_bridge.

## Agreed

None yet.

## Open questions

- Which group protocol: Marmot, OpenMLS directly over our own transport, or p2panda group encryption?
- Which metadata must relays not see: sender, recipient, timing, group size?
