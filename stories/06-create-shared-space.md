# A person can create a shared space naming its services

Status: implemented — single-node; creating the space in p2panda comes with replication

## Calls

```js
const {slug} = await personal.call('POST [@carbon]/space', {name: "Randall's Playhouse", services: ['chat']})
slug   // 'randalls-playhouse'
const rph = personal.ref(`space:${slug}`)
```

## Rules

- A space a person creates for themselves is nested in their personal space, so its ref stays private.
- `POST [@carbon]/space` takes a name shared with members and derives a local slug from it, adding `-N` with an incrementing integer when the slug is taken; an optional `slug` overrides it. It answers the new space's ref. `GET [space]/refs` on the personal space lists them.

- `POST [@carbon]/space` runs in the space of the handle it is called on, so the new space is nested where it was added. Its handler creates the space, in p2panda once replication is built, and writes the space's first op, `POST [space]/origin`, carrying its name, services and manage list.
- `manage` defaults to the calling identity. An empty list means no one may delete the space or add members.
- Anyone may create a space. Attaching it to another space, so that it is visible to and managed by that space's members, needs that space to accept a `POST [space]/ref` first.
- The creator is a member.
- The set of services is fixed at creation; their versions still evolve. Adding services to an existing space is in the [backlog](backlog/add-services-to-space.md).

## Acceptance

- `GET [@carbon]/status` shows the new space's name and services, and the space accepts [chat] calls.
- A call to a service the space does not run is rejected.
