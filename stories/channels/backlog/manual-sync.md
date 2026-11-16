# Ops can travel between devices by any transport, not only p2panda

Status: backlog — set aside until the p2panda implementation

## Idea

A row from another device reaches a space only through `Carbon::receive` (`crates/core/src/carbon.rs`), which only the core's Rust tests call. Exposing it as a sync API on its own, independent of any transport, would make a single device more useful: a tester could play any sequence of ops between two devices from JavaScript, and someone could sync over a transport of their own. p2panda would then be one transport over the same API.

The API would export a service's rows after a cursor on one device and import them on another. It needs three things it does not have today:

- **Signed rows.** An op's author and device are plain fields, so a row delivered by hand could claim any author. Each row would be signed with its device's key and verified when it arrives.
- **Membership checked on receipt.** `receive` checks only the op's time and the space's RunState. A row would be refused unless its author may write the space.
- **Joining a space from its rows.** A device holds a space only when it created it; joining one created elsewhere needs peers today.

## Waiting on it

- **A JavaScript write's `fetch`, replayed.** Rust tests replay a write's recorded `fetch` after a rewind through `[probe]`, over WASI 0.2. JavaScript's `fetch` goes through WASI 0.3, into the same host hook and the same `fetch` effect, but no test replays one: the JavaScript API cannot deliver a late op. Once rows travel between devices, a JavaScript test makes a write that fetches on one device, delivers an earlier op from another, and checks the replay gets the recorded response and sends nothing.

## To decide

- Whether the sync API is CarbonDB's own, with p2panda one transport over it, or p2panda's API is the one apps reach.
- What a row is signed with and what the signature covers: the op alone, or the op and the effects its first run recorded.
- What a device needs to join a space from rows another device sent: its origin, its package, and the members' keys.
