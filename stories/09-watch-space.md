# A person can see changes in a space as they arrive

Status: agreed

## Calls

```js
const stop = rph.watch('GET [chat]/messages', render)
identity.watch('GET [carbon]/invitations', renderInvitations)
```

## Rules

- The app renders tables through GETs, never ops. `watch` runs the GET and re-runs it whenever a committed op writes a table the GET read.
- CarbonDB knows what a GET reads (SQLite's authorizer) and what an op writes (its session changeset), so any GET can be watched.
- A batch of synced ops re-runs a watch once, not once per op.
- Identity-level changes (invitations, devices joining) are GETs in the [carbon] namespace; there is no separate event API.
- Watches re-run on any write to a table they read; narrowing to rows is left for later.

## Acceptance

- A watched `GET [chat]/messages` on Sally's device re-runs when Brandon's message syncs, and not when an unrelated table changes.
- Syncing many ops at once re-runs the watch once.
