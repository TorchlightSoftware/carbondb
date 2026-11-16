# A person can call a service in a shared channel

Status: agreed — the call is built single-node; ordering, rewind and replaying effects are built on a single device; sync between members needs p2panda

## Calls

```js
await rph.call('POST [chat]/messages', {text: 'lunch?'})
await rph.call('GET [chat]/messages')
```

The same `call` as in a personal channel.

## Rules

- A call resolves once this device has applied it, without waiting for other members. A later rejection is reported.
- Reads are answered from this device's copy, even when it is behind other members.
- Every device applies a service's ops in one shared order (hybrid logical clock, device, op hash) and re-runs the service's own handler, so business rules give the same answer everywhere. A late op rewinds only the ops after it that overlap it, in that service only, using SQLite session changesets. See [docs/replication.md](../../docs/replication.md).
- A handler's call to another service in the channel is a message: made on the op's first run, its answer recorded on the op's log row, and every replay gets the recorded answer. Only delivery is guaranteed between services, and atomicity ends at the service: if the caller is refused, what the other service did stays.
- Handlers take time, ids and randomness from the op, never from `Date.now`, `randomUUID` or `Math.random`.
- A handler receives the verified author and device as plain data.

## Acceptance

- Alice posts while Sally is offline; when she syncs, `GET [chat]/messages` on her device returns the message with Alice as author.
- Concurrent edits on two offline devices converge to the same state on both after sync.
- An edit replayed after a concurrent delete is rejected identically on every device.
