# A manager can move a service to a new version of its package

Status: backlog

## Idea

Every op records the hash of the package that handled it, and later the version the package declares (see [packages](../packages.md)). When an app ships a new version of a package, each service a channel names with it has to move to it, and every replica has to agree from which op onward the new version applies. One shape: a package names the versions it supersedes, and moving a service is an op on `[channel]`, so it lands in the shared order like any other.

A device runs one version of a service at a time. After the move, that version decides about ops written under older versions, whether they arrive late or a rewind re-runs them: it supports each one, migrating its input if needed, or refuses it (see [old ops](old-ops.md)).

## To decide

- Whether a package names the versions it supersedes, or the upgrade op names both.
- What a device that lacks the new version does with ops from it: hold the service at that op until the app upgrades, or refuse them, as it refuses an op made for another version today.
- How migrations run across versions.

## Related

- `POST [channel]/services`, which changes a channel's services through the same kind of op ([docs/services.md](../../docs/services.md#channel)).
