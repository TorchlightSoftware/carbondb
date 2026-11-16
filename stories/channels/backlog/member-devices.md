# A member's new device joins their channels

Status: backlog

## Idea

A member of a channel holds it with each of their devices. When they enroll a new device, it has to join every channel they belong to, and each of those channels' services, at the member's access. Adding a device to a channel takes `manage` there, which a member with `write` does not have.

For now, someone with `manage` on the channel has to be online, and is presented with the request to add the new device.

## To decide

- How the request reaches the managers: as an op in the channel, carrying the new device's key signed by the member's identity, which any manager's device can act on.
- What happens when several managers are online and respond: whether adding the same device twice is harmless, or one response must win.
- What the member sees while the device waits: the channels it has joined and those still pending.
- Whether this goes away once p2panda accepts write through a group ([upstream-group-write](upstream-group-write.md)), with each identity added to a channel as a group of its devices.
