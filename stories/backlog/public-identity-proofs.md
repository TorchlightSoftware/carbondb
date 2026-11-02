# Anyone can verify an identity's devices and public proofs

Status: backlog

## Idea

Keybase's sigchain was public, so anyone could check which devices belong to a person and the proofs that bolster their trust and reputation, such as ownership of social media accounts. CarbonDB's identity space has the same shape and should carry the same guarantees, while staying private by default.

## Agreed

- The identity space holds only what may be published: the identity's origin, device public keys, proofs, and one ref to the personal space. Refs to other spaces, device names and tags live in the personal space.
- Device public keys stay public because a proof is signed by a device, and a verifier traces that device back to the origin through `[device]/origin` entries. That reveals how many devices the identity has.
- `GET [@carbon]/identity/export` returns the identity's ops with their CarbonDB signatures. They verify offline, from the origin, whose hash is the identity id. Where to post them, such as a website or nostr, is the app's choice.
- A manual export goes stale and misses later revocations. People who choose manual export accept that for privacy.
- `publish = true` under `[identity]` in the TOML file serves the identity to anyone who subscribes.
- People who share a space with the identity already see its device keys through its p2panda group. Publishing does not duplicate that.

## To decide

- How auto-publish works. The likely mechanism is a plain signed p2panda-core log on its own topic, outside p2panda-spaces, since every p2panda space encrypts its messages. Granting Read to whoever asks was rejected: it needs communication and connectivity, and published facts should be readable without either.
- What a proof is: its call, its fields, and how a verifier checks the external claim.
- Whether other replicas should also verify device authorization from the identity log, rather than relying only on the p2panda group.

## Related

- [CarbonDB gets a security and privacy review after the prototype](security-review.md)
