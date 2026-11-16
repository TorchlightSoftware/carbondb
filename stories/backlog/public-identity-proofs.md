# Anyone can verify an identity's devices and public proofs

Status: backlog

## Idea

Keybase's sigchain was public, so anyone could check which devices belong to a person and the proofs that bolster their trust and reputation, such as ownership of social media accounts. CarbonDB's identity channel has the same shape and should carry the same guarantees, while staying private by default.

## Agreed

- An earlier design kept the identity's own channel to what may be published: its origin, device public keys, proofs, and one ref to the personal channel. `[identity]` now also keeps the devices' private names and tags beside their keys ([docs/channels.md](../../docs/channels.md#an-identitys-two-channels)), so what a published record holds is to decide again.
- Device public keys stay public because a proof is signed by a device, and a verifier traces that device back to the origin through the devices `[identity]` lists. That reveals how many devices the identity has.
- `GET [@carbon]/identity/export` returns the identity's ops with their CarbonDB signatures. They verify offline, from the origin, whose hash is the identity id. Where to post them, such as a website or nostr, is the app's choice.
- A manual export goes stale and misses later revocations. People who choose manual export accept that for privacy.
- `publish = true` under `[identity]` in the TOML file serves the identity to anyone who subscribes.
- People who share a channel with the identity see its device keys through the services they share in p2panda. Publishing does not duplicate that.

## To decide

- What a published record holds, now that `[identity]` keeps device names and tags beside the keys.
- How auto-publish works. The likely mechanism is a plain signed p2panda-core log on its own topic, outside p2panda-spaces, since every p2panda space encrypts its messages. Granting Read to whoever asks was rejected: it needs communication and connectivity, and published facts should be readable without either.
- What a proof is: its call, its fields, and how a verifier checks the external claim.
- Whether other replicas verify a device's authorization from `[identity]`, rather than relying only on p2panda membership ([p2panda-sync.md](../p2panda-sync.md)).

## Related

- [CarbonDB gets a security and privacy review after the prototype](security-review.md)
