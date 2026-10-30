# A person can call a service in their personal space

Status: agreed

## Calls

```js
const personal = await identity.createSpace({name: 'Personal', services: ['contacts', 'chat']})
// later sessions
const personal = await identity.resumeSpace('personal')

await personal.call('PUT [contacts]/contacts/sally', {name: 'Sally'})
await personal.call('GET [contacts]/contacts')
```

## Rules

- A personal space is an ordinary space that starts with one member. It may be shared later (a caretaker, power of attorney); warning before sharing is up to the app.
- `createSpace` takes a name shared with members and derives a local slug from it, adding `-N` with an incrementing integer when the slug is taken; an optional `slug` overrides it. `identity.listSpaces()` lists them.
- `call` resolves with the handler's result or rejects with its error. A call refused on this device is not published.
- A GET is answered from this device's copy and never becomes an op.
- Each call runs in one transaction: queries return results immediately, and the writes commit when the handler resolves or roll back when it fails.

## Acceptance

- A PUT followed by a GET returns the new record, and both survive a restart.
- A call whose handler returns an error leaves no rows and publishes nothing.
