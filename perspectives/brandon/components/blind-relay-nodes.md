# Blind Relay Nodes

Critical path: yes. Depends on: none.

## Context

- Folds in the wiki terms Quality of Service and Reputation, as spam resistance and relay incentives.
- Wiki: relays should hold messages while the destination is offline; retention depends on node policy and user compensation; anonymity via throwaway identities, per-chunk relays, trusted peers, or privacy networks; relays may charge fees.
- nostr relays store and forward ciphertext natively; NIP-42 AUTH limits who can fetch gift wraps; relays still see IP and timing.
- Freenet: room state persists on hosting peers while someone wants it; cold data can disappear from every host.
- Holochain: iroh relays forward live QUIC; no store-and-forward; offline delivery depends on DHT peers being online.
- Autonomi x0x: QUIC NAT traversal and relays; offline store-and-forward is undocumented.
- Ockam: relays forward encrypted traffic without store-and-forward; its hosted relays appear to be gone.
- iroh: QUIC with NAT hole-punching and relay fallback; defaults to n0-run discovery servers and public relays; iroh-relay and iroh-dns-server are published for self-hosting; relays forward live traffic only, no store-and-forward *(inferred)*.
- p2panda offline delivery needs an always-on peer to sync through *(inferred)*.
- Delta Chat uses iroh 0.35 for realtime peer channels.
- Spam and Sybil resistance, nostr: NIP-13 proof of work, NIP-42 AUTH, paid and web-of-trust relays, NIP-85 Trusted Assertions.
- Spam and Sybil resistance, others: Autonomi has EigenTrust node scoring and ANT rewards but no user-level spam control; Holochain uses membrane proofs and warrants designed per app; Freenet has per-peer rate limits and donation-backed antiflood tokens, with peer location from IP prefix an eclipse weakness.

## Agreed

None yet.

## Open questions

- Is a relay operator seeing IP and timing acceptable, or do we need onion-style routing?
- Who runs relays, and are they paid per message (zaps or ecash), by subscription, or run by the community?
- Run our own iroh relays and discovery servers rather than the n0 defaults?
- Can an always-on peer that syncs p2panda logs fill the Blind Relay Nodes role for offline phones?
