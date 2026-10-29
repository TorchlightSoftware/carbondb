# Device Constellation

Critical path: yes. Depends on: [Identity](identity.md), [State Synchronization](state-synchronization.md).

## Context

- Wiki goals: sync between devices, keep access after losing a device, recover via a secondary device or a paper backup key, and revoke every device if all are stolen.
- Wiki prior art: Keybase QR enrollment (acquired by Zoom) and Holochain DeepKey; DeepKey has since been removed from Holochain. Keybase, Matrix QR login and KERI are detailed under [Identity](identity.md).
- No candidate network ships a Device Constellation.
- nostr: NIP-46 remote signer; Marmot gives each device its own MLS member under one pubkey; the paper backup is the nsec itself or a NIP-49 ncryptsec.
- Freenet: syncing delegates across devices is an open item; today it is a manual encrypted bundle export.
- Autonomi x0x: owner key, agent keys and a key pinned per machine; no recovery.
- Ockam: rotation and revocation exist, but adding devices under one identity and paper-key recovery are not specified.
- p2panda groups (spaces) support multiple devices per member.

## Agreed

- Device enrollment follows Keybase's ergonomics: scan a QR code on an existing device to enroll a new one. We rebuild this, since no candidate provides it.

## Open questions

- Are device keys delegated subkeys signed by the identity root, or is one root key copied to every device?
- Where does the device chain live without a central server, given Keybase kept it on keybase.io?
- What is the recovery path: secondary device, paper key, social recovery, or an optional custodial service?
- Is the paper key a full-powered device, as in Keybase?
- Does the wallet key join the same constellation, or does the wallet keep its own seed?
