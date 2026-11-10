# A person can see changes in a space as they arrive

Status: implemented — single-node; batching synced ops needs replication

## Calls

```js
const stop = rph.watch('GET [chat]/messages', render)
personal.watch('GET [device]/refs', renderDevices)
```

## Rules

- The app renders tables through GETs, never ops. `watch` runs the GET and re-runs it whenever a committed op writes a table the GET read.
- CarbonDB knows what a GET reads (SQLite's authorizer) and what an op writes (its session changeset), so any GET can be watched.
- A batch of synced ops re-runs a watch once, not once per op.
- A write's caller hears back once the watches its commit owes a run have run again, so a watched view already shows the write. A GET answers at once.
- When the GET fails, `fn` receives `undefined` and the error, and the watch runs again after the next commit that writes what it read.
- Identity-level changes are GETs, such as devices joining in [device]; there is no separate event API.
- Watches re-run on any write to a table they read; narrowing to rows is left for later.

## Acceptance

- A watched `GET [chat]/messages` on Sally's device re-runs when Alice's message syncs, and not when an unrelated table changes.
- Syncing many ops at once re-runs the watch once.
