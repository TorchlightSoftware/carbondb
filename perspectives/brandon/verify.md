# Things to verify

Claims the design leans on that nobody has confirmed yet. Each says what we believe, why it matters, and how to check it. p2panda references are to the commit pinned in `spikes/p2panda-demo/Cargo.toml` (`da80c05`).

## p2panda

- **Membership order agrees with ours.** Adding or removing a member is a [carbon] call at a position in our shared order, and it also changes p2panda's group state, which p2panda resolves by its own rules (by default a concurrent removal wins). Both must give the same answer to "was this identity a member when it made this call?". Check: a test with a removal concurrent with a call by the removed member, comparing p2panda's resolved membership with our order's.
- **Receivers re-check authorship.** `Space::publish` checks `can_write` on the author's side (`p2panda/src/spaces/space.rs:152-156`). Unknown whether a receiving peer rejects a message from an author who was not a writer at that point. Check: a removed member publishes, and a remaining member must not apply it.
- **Acknowledge only after our commit.** p2panda's stream controller replays operations the application has not acknowledged after a crash ([2025-07-09 post](https://p2panda.org/2025/07/09/streams-transactions-crash-resilience.html)). We should acknowledge an op only once our apply has committed it to the space's SQLite. Check that the acknowledgement API at the pinned commit allows this.
- **Devices under an identity.** We model an identity as a signed log of its devices, Keybase style. p2panda-spaces offers nested groups for multi-device profiles. Check how a group of an identity's devices joins a space, and how revoking a device propagates.
- **Catching up a new member without all history.** `PruneFlag` lets an author drop everything earlier in its own log (`p2panda-core/src/prune.rs:3-11`). Unknown how to hand a new member a snapshot of state many authors wrote.
- **Operation size.** Gossip defaults to 4096 bytes per message (`p2panda-net/src/gossip/config.rs:4`); limits on log sync and operation bodies are unknown. Matters for large inputs, such as a file import.

## SQLite and the binding

- **One SQLite library in the binary.** Our databases use rusqlite with its `session` feature; p2panda-store uses sqlx. Both link `libsqlite3-sys`, and Cargo allows one version per build. Check that compatible versions resolve, or use one library for both.
- **Sessions under real handlers.** rusqlite 0.41 has `Changeset::invert` and an authorizer that reports reads by table and column. Check changesets and inversion with upserts, `RETURNING`, and deletes, and that the authorizer sees every read.
- **Awaiting JS from Rust.** The core calls a JS handler, keeps the op's transaction open while the handler's Promise is pending, and serves the handler's synchronous database calls on the same connection. Check that napi-rs supports this without deadlock.
