# A person can call a service in their personal channel

Status: implemented — single-node

## Calls

```js
// created with the identity (story 1)
await carbon.call('POST [@carbon]/identity', {slug: 'alice', device: 'laptop', services: ['contacts', 'chat']})
// this session and later ones
const personal = carbon.ref('identity:alice').ref('personal:alice')

await personal.call('PUT [contacts]/contacts/sally', {name: 'Sally'})
await personal.call('GET [contacts]/contacts')
```

## Rules

- Every identity has one personal channel, created with it and nested in its own channel. It is where everything else of the person's nests: the refs to channels they create or join, and personal data. It starts with one member and may be shared later (a caretaker, power of attorney) without being conflated with the identity; warning before sharing is up to the app. Device names and tags are not here: they are in `[identity]`, in the identity's own channel.
- `[channel]` always runs there. Its other services are named when the identity is created, or added later with `POST [channel]/services` ([story 6](06-create-shared-channel.md)).
- A call on the handle reaches the service the channel names `[name]`; a name the channel does not name gets a 404.
- `call` resolves with the handler's result or rejects with its error. A call refused on this device is not published.
- A GET is answered from this device's copy and never becomes an op.
- Each call runs in one transaction in its service's own file: queries return results immediately, and the writes commit when the handler resolves or roll back when it fails. A call it makes to another service in the channel is a message, made once and recorded; what that service did stays even when this call then fails (see [docs/replication.md](../../docs/replication.md#effects-are-made-once)).

## Acceptance

- A PUT followed by a GET returns the new record, and both survive a restart.
- A call whose handler returns an error leaves no rows and publishes nothing.
