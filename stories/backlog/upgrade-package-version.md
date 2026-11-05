# A manager can move a space to a new version of a service

Status: backlog

## Idea

Every op records the hash of the package that handled it and its migration counts (see [packages](../packages.md)). When an app ships a new version of a service, a space has to move to it, and every replica has to agree from which op onward the new version applies. One shape: a package names the versions it supersedes, and moving a space is an op on `[space]`, so it lands in the shared order like any other.

## To decide

- Whether a package names the versions it supersedes, or the upgrade op names both.
- What a device that lacks the new version does with ops from it: it holds the space at that op until the app upgrades, as with a newer schema today.
- How migrations run across versions, and whether ops from the old version remain valid after the upgrade op.
- How this meets [old ops](old-ops.md): a rewind across an upgrade re-runs old ops with the old version, so a replica needs both.

## Related

- [Adding a service to a space](add-services-to-space.md), which changes a space's services through the same kind of op.
