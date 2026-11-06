# Contacts

Critical path: yes. Depends on: none.

## Context

- Folds in the wiki terms Peer Discovery and Distributed Hash Table.
- Wiki: Contacts makes use of Peer Discovery and Reputation; Peer Discovery uses a DHT plus a bootstrap server or a bootstrap list (file, QR code, peer to peer).
- nostr: follow lists, NIP-05 DNS names, npub QR codes; clients talk only to relays, routed by outbox relay lists.
- KeyOxide: lookup by fingerprint or email through WKD, keyservers or ASPE, all plain HTTP.
- Autonomi: four-word addresses are shipping; Kademlia DHT; x0x bootstraps from nodes the team runs in 6 cities.
- Holochain: bootstrap server plus Kitsune2 gossip; contacts are exchanged out of band.
- Freenet: invite links into rooms only; small-world ring routing after a gateway bootstrap.
- iroh resolves a known key to its current address via signed DNS or Pkarr records and local mDNS; it does not find the key for a person.
- p2panda has confidential topic discovery, and is designed to also work over BLE, LoRa or USB, which matches the wiki Mesh Network hope.
- A petname such as {frank} is a local, subjective name for a key in the address book; nostr NIP-02 follow lists carry a petname field.
- Petnames resolve the secure, decentralized, human-readable trade-off (Zooko's triangle) by being meaningful only to their holder.

## Agreed

None yet.

## Open questions

- Human-readable names (NIP-05-style DNS), or keys plus QR exchange only?
- Must discovery work on a local mesh without internet, as the wiki Mesh Network page hopes?
- Are petnames display-only, with stored and shared data (messages, automation queries) always holding the key?
