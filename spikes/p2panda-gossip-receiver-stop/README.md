# p2panda PR: stop the gossip receiver when its event stream ends

A ready-to-file issue and pull request for p2panda: p2panda-net's gossip receiver actor polls iroh-gossip's event stream again after it has ended, which panics. The fix stops the actor when the stream ends; a new test fails without it.

- Base: p2panda `main` at `b290910107f27219d290f6764dbdb1b2a42bad48` (2026-11-16).
- Change: [`gossip-receiver-stop.patch`](gossip-receiver-stop.patch), touching `p2panda-net/src/gossip/actors/receiver.rs` (fix and test) and `CHANGELOG.md`.
- Nothing has been posted to GitHub.

## To file it

1. Open the issue below on p2panda/p2panda.
2. In a fork of p2panda at the base commit (or rebased onto current `main`): `git apply gossip-receiver-stop.patch`, commit, push, and open the PR below.
3. Replace both `#PR` placeholders in the `CHANGELOG.md` line with the PR's number, and `#ISSUE` in the PR text with the issue's.

## Issue

**Title:** Gossip receiver actor panics when polled after its event stream ended

**Body:**

When iroh-gossip's event stream for a topic ends, for example because the iroh `Gossip` instance shuts down while a node is dropped, p2panda-net's gossip receiver actor can panic:

```
thread '<unnamed>' panicked at futures-lite-2.6.1/src/stream.rs:509:14:
`Unfold` must not be polled after it returned `Poll::Ready(None)`
```

In `p2panda-net/src/gossip/actors/receiver.rs`, `ToGossipReceiver::WaitForEvent` handles `Some(Ok(event))` and `Some(Err(_))`, but when `receiver.next().await` returns `None` it falls through and casts `WaitForEvent` to itself again. The next `next()` polls the ended stream; iroh-gossip's `GossipReceiver` wraps an `irpc` receiver turned into a stream with `n0_future::stream::unfold`, which is not fused, so that poll panics.

ractor catches the panic, so the receiver actor fails and its gossip session sees an `ActorFailed`, but the panic is printed to stderr and a build with `panic = "abort"` would end the process. We saw it while dropping nodes that were still subscribed to topics, about once in 20 runs, since it only happens when the stream ends before the actor is stopped.

The fix is to stop the actor when the stream ends, as `GossipListener` already does when its channel closes. Happy to open a PR with the fix and a test that reproduces the panic deterministically.

## Pull request

**Title:** Stop gossip receiver actor when its event stream ends

**Body:**

When iroh-gossip's event stream ends, `GossipReceiver` cast `WaitForEvent` to itself again, and polling the ended, unfused stream panicked (`Unfold` must not be polled after it returned `Poll::Ready(None)`). It now stops itself when the stream ends, as `GossipListener` does when its channel closes, and as #1598 does for sync actors on completion.

The new test `gossip::actors::receiver::tests::stops_when_the_gossip_event_stream_ends` joins two iroh gossip instances on one topic, gives one side's receiver to a `GossipReceiver` supervised by a stand-in session actor, shuts that side's iroh gossip down, and asserts the receiver terminates rather than fails. Without the fix it fails every time, with the panic above.

Closes #ISSUE

## 📋 Checklist

- [x] Add tests that cover your changes
- [x] Add this PR to the _Unreleased_ section in `CHANGELOG.md`
- [x] Link this PR to any issues it closes
- [x] New files contain a SPDX license header (no new files)

## Evidence

Run in `p2panda/` with `cargo test -p p2panda-net --all-features`, 2026-11-18:

| Check | Result |
|---|---|
| New test, without the fix | fails: the receiver fails (`left: Some(Failed)`, `right: Some(Stopped)`), with the panic above |
| New test, with the fix | passes, 5 runs of 5 |
| p2panda-net's whole suite, with the fix | passes (46 passed, 1 ignored, plus integration and doc tests) |
| `cargo fmt -- --check` | clean |
| `cargo clippy -p p2panda-net --all-features -- -D warnings --no-deps` (as CI runs it) | clean; with `--tests` too, nothing in the changed file |

## The working tree

`p2panda/` (ignored by git) is the base commit with the patch applied, built with `CARGO_TARGET_DIR` set to `../../target/p2panda-upstream/target` from the repository root. To rebuild it: download `https://codeload.github.com/p2panda/p2panda/tar.gz/b290910107f27219d290f6764dbdb1b2a42bad48`, extract it here as `p2panda/`, and `git apply gossip-receiver-stop.patch` inside it.
