# p2panda lets a device write through a group it belongs to

Status: backlog

## Idea

p2panda's high-level `Space` API checks the calling device against a space's direct members before it publishes, adds, removes, promotes or demotes (p2panda 0.8.1, `p2panda/src/spaces/space.rs`). A device whose access comes only through a group it belongs to is refused before anything is sent, though receivers accept access held through a group.

CarbonDB adds devices to each space directly for now, behind its identity-level API, so it does not depend on this. Once that implementation runs, we can describe our use case to p2panda: an identity as a group of devices, with an assignment policy deciding which devices hold each space.

## To decide

- Whether to ask p2panda to accept group-held access in the high-level API, or keep direct device membership.
- What to share upstream about the assignment policy and its device and service tags.
