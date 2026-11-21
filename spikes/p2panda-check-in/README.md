# p2panda check-in

When a node starts holding a space, p2panda-net 0.8.1 finds the nodes that hold it too only through discovery's random walk, even when it is already talking to one of them. CarbonDB chains a new device's channels (alice's identity channel, then the personal channel it points to), so a new device waits on that walk once per channel. This spike patches p2panda-net so a node that starts holding a topic runs discovery at once with every node it already knows how to reach, and measures the wait with and without it.

## Result

With the check-in, the laptop has each space's first message in 0.02–0.07s, every run; as released it waits 1–4s on fresh or idle nodes and up to 13s with a bootstrap node that fails every session, as CarbonDB's milestone test has.

| Setup | Runs | Released: first space | Released: chained space | Check-in: first space | Check-in: chained space |
|---|---|---|---|---|---|
| Fresh nodes | 20 | median 3.03s, slowest 3.03s | median 2.13s, slowest 3.79s | median 0.03s, slowest 0.03s | median 0.03s, slowest 0.03s |
| Idle 60s first | 5 | median 3.01s, slowest 3.19s | median 2.49s, slowest 2.98s | median 0.07s, slowest 0.07s | median 0.03s, slowest 0.03s |
| Bootstrap on another network | 10 | median 3.02s, slowest 7.54s | median 4.32s, slowest 13.32s | median 0.03s, slowest 0.05s | median 0.03s, slowest 0.03s |

Measured 2026-11-18 on one machine, all nodes behind an iroh relay on localhost. The first space improves too: creating spaces makes the phone check in with the laptop, whose address it has, so the laptop knows the phone before it holds the first space.

The minute-long waits CarbonDB's relay milestone sometimes shows did not appear here; the longest wait as released was 13s.

## Why the wait

The chained wait comes from the unrelated bootstrap node: subscribing resets discovery's walkers, a reset walk starts at a bootstrap node, and this one holds neither space, so the walker backs off 1–5s before reaching the phone. The first space's steady 3s comes from walkers that start with an empty address book, sleep 2s and miss the resets sent meanwhile. Both reproduce in p2panda's own terms; see [`../p2panda-discovery-check-in`](../p2panda-discovery-check-in/README.md#why-the-wait).

## The scenario

Each run starts three nodes behind the relay: alice's phone, her new laptop, and an unrelated node both name as a bootstrap, which the random walk may visit. The phone registers the laptop's key bundle and address, as scanning its code does, creates two spaces with the laptop a member, and publishes a message in each. The laptop then holds the first space, and once its message arrives, the second. Each wait runs from the laptop subscribing to the space to the phone's message arriving.

## The patch

`p2panda-net/` is p2panda-net 0.8.1 as published, used through `[patch.crates-io]`, with one change behind its `check-in` feature, in `src/discovery/actors/manager.rs`: when the node's own topics grow, besides restarting the walkers (as released), the discovery manager runs a session at once with every node in its address book that has transport info and is not stale. Those sessions are the ordinary discovery sessions, so each side learns only the topics both hold. Starting a session moved into one function shared by walker steps and check-ins, and a session's walker became optional. Without the feature the crate behaves as released. `diff -ru` against the published crate shows the change.

## Run

```
cargo run --release -- <runs> [idle seconds] [foreign]                     # as released
cargo run --release --features check-in -- <runs> [idle seconds] [foreign] # with the check-in
```

`idle seconds` pass after the phone sets up both spaces and before the laptop holds the first; `foreign` runs the bootstrap node on p2panda's default network. Set `RUST_LOG=p2panda_net=debug` for p2panda's logs.

## Seen along the way

- Every dropped node logs `Endpoint dropped without calling Endpoint::close`: p2panda 0.8.1's `Node` has no shutdown.
- Once in 20 check-in runs, a p2panda thread panicked while nodes were dropped (``Unfold` must not be polled after it returned `Poll::Ready(None)``, in futures-lite); the run's measurements were unaffected. Not traced.
