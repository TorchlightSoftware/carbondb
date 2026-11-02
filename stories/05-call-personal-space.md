# A person can call a service in their personal space

Status: agreed

## Calls

```js
// created with the identity (story 1)
const {identity, personal} = await carbon.createIdentity({slug: 'brandon', device: 'laptop', services: ['contacts', 'chat']})
// later sessions
const personal = carbon.ref('identity:brandon').ref('personal:brandon')

await personal.call('PUT [contacts]/contacts/sally', {name: 'Sally'})
await personal.call('GET [contacts]/contacts')
```

## Rules

- Every identity has one personal space, created with it. It is the private root of what the identity owns: the refs to spaces it creates or joins, its devices' names and tags, and personal data. It starts with one member and may be shared later (a caretaker, power of attorney) without being conflated with the identity; warning before sharing is up to the app.
- Its services are named when the identity is created, since a space's services are fixed at creation. `[space]` and `[device]` always run there.
- `call` resolves with the handler's result or rejects with its error. A call refused on this device is not published.
- A GET is answered from this device's copy and never becomes an op.
- Each call runs in one transaction: queries return results immediately, and the writes commit when the handler resolves or roll back when it fails.

## Acceptance

- A PUT followed by a GET returns the new record, and both survive a restart.
- A call whose handler returns an error leaves no rows and publishes nothing.
