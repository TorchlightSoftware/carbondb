# Identity

Critical path: yes. Depends on: none.

## Context

- Folds in the wiki terms Self-Sovereign Identity, Public & Secret Key, User Keychain and Public Claims.
- Wiki: anyone can generate their own identity without brokers or gatekeepers, may hold several identities for anonymity, and enrolls devices by QR code to form a Device Constellation.
- nostr: one permanent secp256k1 key; no shipped rotation or revocation NIP; NIP-06 multi-key derivation is now marked unrecommended, so there is no User Keychain.
- Holochain: Ed25519 agent key per app held in the Lair keystore; DPKI and Deepkey were removed from the conductor in 0.6, so there is no rotation.
- KeyOxide ASP: Ed25519 or P-256 signed JSON profile whose identifier is the key fingerprint; updates must be signed by the same key; no rotation or revocation. OpenPGP profiles do have subkeys and revocation.
- Ockam: the identifier is the hash of the first signed key change; a signed change history rotates the primary key and revokes purpose keys; purpose keys are attested by the identity key. It is the only candidate with rotation designed in.
- Autonomi: post-quantum ML-DSA-65 and ML-KEM-768 keys; unlimited HKDF child keys from one master secret; docs state there is no recovery mechanism.
- Freenet: no global identity; River uses a key per room; delegates keep keys on the device.
- iroh dials Ed25519 public keys rather than IP addresses.
- Prior art, Keybase: two systems. Account proofs, which KeyOxide rebuilt; and a signature chain of per-device keys, which KeyOxide never built.
- Prior art, Keybase device chain: each device generates its own key; an existing device signs the new one into the chain without re-entering a passphrase; a paper key is a full-powered device key; revoking a device rotates the per-user key.
- Prior art, Keybase code: keybase/client is BSD-3-Clause and still committed to (pushed 2026-10-26); the keybase.io server that stored every signature chain was never open source.
- Prior art, Matrix: QR login (MSC4108) lets a signed-in device vouch for a new one and hand it the cross-signing keys; it depends on homeserver sign-in (MAS), so only the user experience transfers.
- Prior art, KERI: a key event log with pre-rotation (committing to the next key in advance) so a stolen current key cannot take over the identity; keripy is Apache-2.0 and active, the Rust keriox has been idle since 2023-01.
- Wiki Public Claims: signed statements anyone can verify; agreements can be witnessed and kept off ledger until a dispute.
- KeyOxide does two-way account linking; verification fetches live third-party pages, some through proxy servers.
- nostr has NIP-05 and external identity proofs; Holochain has signed source-chain entries validated by peers.

## Agreed

None yet.

## Open questions

- Which format records the identity root's key history: a Keybase-style signature chain, a KERI key event log, or an Ockam-style change history?
- If transport is nostr, how does a rotated identity map onto a new npub that contacts already follow, without breaking their follow lists?
- Classical keys (Ed25519, secp256k1) or post-quantum, given wallet tooling expects classical curves?
- Use KeyOxide ASP profiles as they are, or only borrow the claim format onto our own identity root?
- Fork any Ockam crates, or only reimplement its identity design?
- Is account linking (Public Claims) needed at launch, or later?
