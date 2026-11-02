# A person can revoke a lost device

Status: backlog

## Idea

A device that is lost or stolen is revoked from its identity: removed from the identity's p2panda group, which rotates the group secrets of every space the identity is in, and recorded in the identity space so an exported chain shows the revocation.

## To decide

- The call, and the entries it writes in the identity space and the personal space.
- What happens to the revoked device's ops above the point of revocation, by analogy with a member's removal cutoff.
- Two devices that revoke each other concurrently are both removed by p2panda, and nothing keeps a last manager. Whether a paper key or a hardware key must authorize device changes (see the [security review](security-review.md)).

## Related

- [A manager can remove a member, and a member can leave or delete a space](remove-member.md)
