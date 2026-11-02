# A manager can remove a member, and a member can leave or delete a space

Status: backlog

## Already set

These rules are in [implementation.md](../implementation.md#removing-a-member):

- A removal op carries the highest op the remover's device has seen from each device of the removed member. Every replica discards that member's ops above the cutoff, and rewind cleans up what depended on them.
- Discarding a removed member's unseen offline work is required behaviour.
- Removing someone from a company removes them from its teams, through `cascade`.

## To decide

- What the calls look like: removing a member, leaving, and deleting a space.
- Each team removal is a separate op with its own cutoff, and the remover needs manage in every team, so a cascade is not atomic. What happens if a company-wide removal reaches only some teams?
- What deleting a space does on each device, given that p2panda has no per-space delete.

## Related

- [A person can revoke a lost device](revoke-device.md)
- [Replicas are proven to hold the same ops when a member is removed](verify-replica-op-sets.md)
