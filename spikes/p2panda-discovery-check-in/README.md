# p2panda PR: check in with known nodes on a new topic

**Reproduced; the issue is not drafted yet.** The change, its unit test and a reproduction in p2panda's own terms are ready: on main a node waits 0.7–4s for a second space it shares with a peer it already syncs with, whenever a bootstrap node holds neither space; with the change, 0.3–0.5s. Before drafting the issue, decide whether it reports the causes below and offers the check-in as one fix, or proposes fixing the reset itself.

- Base: p2panda `main` at `b290910107f27219d290f6764dbdb1b2a42bad48` (2026-11-16).
- Change: [`discovery-check-in.patch`](discovery-check-in.patch): when a node's own topics grow, its discovery manager runs a session at once with every reachable, non-stale node in its address book, besides restarting the walkers as it does today. Touches `p2panda-net/src/discovery/actors/manager.rs`, `p2panda-net/src/discovery/tests.rs` and `CHANGELOG.md`.
- Reproduction: [`repro/second_space.rs`](repro/second_space.rs), a test for the `p2panda` crate's `tests/`, failing on main and passing with the change.
- Nothing has been posted to GitHub.

## What is ready

The patch applies to the base commit. Its test, `discovery::tests::subscribing_to_a_topic_checks_in_with_known_nodes`, gives two nodes each other's addresses and no random walkers, so only a check-in can start a discovery session: without the change Bob never learns Alice shares the topic (it times out after 5s); with it, both learn within about 0.17s, 5 runs of 5. `cargo fmt -- --check` is clean, and so is CI's `cargo clippy -p p2panda-net --all-features -- -D warnings --no-deps`.

p2panda-net's whole suite is flaky on main itself: over 8 runs each, main failed 3 (`sync::log_sync::tests::e2e_three_party_sync` twice, `gossip::tests::join_without_bootstrap` once) and main with the change failed 2 (`e2e_three_party_sync`, `iroh_mdns::tests::mdns_network_separation`).

## The reproduction

[`repro/second_space.rs`](repro/second_space.rs) is the scenario in p2panda's terms: panda holds two spaces with penguin a member of both; penguin syncs the first, the nodes run 3s more, then penguin subscribes to the second, and the test measures how long the second space's message takes. Nodes find each other only through a relay on localhost, with mDNS off. One test runs two nodes, the other adds a third node both use as a bootstrap, which holds neither space. Run 2026-11-19, 6 times each:

| Test | main | main with the change |
|---|---|---|
| Two nodes | passes, 0.34–0.39s | passes, 0.35–0.45s |
| With a bootstrap node | fails, 0.73–4.05s | passes, 0.32–0.46s |

## Why the wait

Read from logs of each walker's steps and from the code, on main:

1. **A reset walk starts at a bootstrap node.** Subscribing to a topic resets the walkers (`ResetWalkers`, from the topic watcher in `p2panda-net/src/discovery/actors/manager.rs`), and a reset walk asks for a random bootstrap node first (`p2panda-discovery/src/random_walk.rs`, `next_node` and `random_bootstrap_node`). A bootstrap node that holds neither space returns no topic in common and nothing new, so the walker backs off a random 1–5s (`discovery/backoff.rs`) before its next step reaches panda. Without bootstrap nodes the walk picks any known node, here panda, and the second space syncs at once.
2. **A reset is dropped unless a walker is waiting in its backoff.** `ResetWalkers` calls `Notify::notify_waiters` (`manager.rs`), which wakes only walkers parked in the `select!` on their backoff (`walker.rs`). A walker in its 2s sleep after finding no node (`NO_RESULTS_DELAY`), or waiting on a session, misses the reset and keeps its old schedule. That is the fresh-node wait: in the two-node test without the 3s settle, the second space took 0.94s with no pause before subscribing and 0.34–0.42s once the pause passed the walker's sleep (swept 0–2.25s). It is also the first space's steady ~3s: both nodes' walkers start with an empty address book and sleep 2s, missing the resets of creating and subscribing.

The check-in removes both: it does not depend on the walkers. Neither the runtime flavour (single- or multi-threaded tokio, same waits) nor nodes left from earlier runs in one process (a single run per process waits as long) matters.

An earlier batch (2026-11-18) measured the bootstrap case at 0.31–0.37s on main, 10 runs; it did not recur in about 30 runs on 2026-11-19 and is unexplained. Its two-node runs (0.25–0.39s) fit the settled case above.

## The working tree

`p2panda/` (ignored by git) is the base commit with the patch applied, built with `CARGO_TARGET_DIR` set to `../../target/p2panda-upstream/target` from the repository root. To rebuild it: download `https://codeload.github.com/p2panda/p2panda/tar.gz/b290910107f27219d290f6764dbdb1b2a42bad48`, extract it here as `p2panda/`, and `git apply discovery-check-in.patch` inside it. To run the reproduction, copy `repro/second_space.rs` into `p2panda/p2panda/tests/`, add `iroh-relay = { version = "1.4.1", features = ["server"] }` to `p2panda/p2panda/Cargo.toml`'s dev-dependencies, and run `cargo test -p p2panda --test second_space -- --nocapture`; against main, apply nothing.
