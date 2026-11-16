# p2panda lets a device write through a group it belongs to

Status: backlog

## Idea

p2panda's high-level `Space` API checks the calling device against a space's direct members before it publishes, adds, removes, promotes or demotes (p2panda 0.8.1, `p2panda/src/spaces/space.rs`). A device whose access comes only through a group it belongs to is refused before anything is sent, though receivers accept access held through a group.

p2panda's intended shape is an identity as a group of its devices: a channel adds the group once, and the identity adds its own new devices to its group, with no manager of the channel involved. Because write through a group is refused, CarbonDB adds each device to each p2panda space directly, so a device with `manage` must act whenever a member's devices change. That was already so when services shared a space; with channels it costs more, since each service in a channel is its own p2panda space and a device must join every one.

Once group write works, two things shrink:

- [A member's new device joining their channels](member-devices.md) needs no manager online: the member adds the device to their own group.
- An invitation no longer needs an invitation key standing in for the invitee until a manager replaces it: the channel adds the invitee's group once.

CarbonDB adds devices directly for now, behind its identity-level API, so it does not depend on this. Once that runs, we can describe our use case to p2panda with a working example.

## To decide

- Whether to ask p2panda to accept group-held access in the high-level API, or keep direct device membership.
- What to share upstream about the assignment policy and its device and service tags.
