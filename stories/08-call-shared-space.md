# A person can call a service in a shared space

Status: agreed

## Calls

```js
await rph.call('POST [chat]/messages', {text: 'lunch?'})
await rph.call('GET [chat]/messages')
```

The same `call` as in a personal space.

## Rules

- A call resolves once this device has applied it, without waiting for other members. A later rejection is reported.
- Reads are answered from this device's copy, even when it is behind other members.
- Every device applies ops in one shared order (hybrid logical clock, device, op hash) and re-runs the service's own handler, so business rules give the same answer everywhere. A late op rewinds only the ops after it that overlap it, using SQLite session changesets.
- Handlers take time, ids and randomness from the op, never from `Date.now`, `randomUUID` or `Math.random`.
- A handler receives the verified author and device as plain data.

## Acceptance

- Brandon posts while Sally is offline; when she syncs, `GET [chat]/messages` on her device returns the message with Brandon as author.
- Concurrent edits on two offline devices converge to the same state on both after sync.
- An edit replayed after a concurrent delete is rejected identically on every device.
