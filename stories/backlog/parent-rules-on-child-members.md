# A company can allow or prevent adding a member to its team

Status: backlog

## Idea

Adding someone to a child space does not add them to its parent. A parent may still want a say, for example a company that allows only its employees or approved consultants into its teams. Those rules would come from RBAC in the parent, such as an app's role service, which CarbonDB does not know about.

## To decide

- Where the parent's rules live, and what form they take.
- Who checks them when a member is added to the child. A handler cannot read another space deterministically, so a child's replicas cannot consult the parent's state while applying an op.
- Whether the rule is enforced on every replica or only on the adding device.
- How this relates to `manage` entries naming a company, which are already copied into the child as the identities they expand to.
- Whether a space can tell that a local service wrote an op, for a company that wants to ban unapproved local services. Peers see only a signed op from a member device, so today a space can only apply its rules to the op itself.

## Related

- [A manager can move a space to a different parent](reassign-space-parent.md)
