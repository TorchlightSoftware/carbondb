# A person can create a shared space naming its services

Status: agreed

## Calls

```js
const rph = await personal.createSpace({name: "Randall's Playhouse", services: ['chat']})
rph.slug   // 'randalls-playhouse'
```

## Rules

- A space a person creates for themselves is nested in their personal space, so its ref stays private.
- `createSpace` takes a name shared with members and derives a local slug from it, adding `-N` with an incrementing integer when the slug is taken; an optional `slug` overrides it. `personal.listSpaces()` lists them.

- `createSpace` calls `POST [@carbon]/space` in the context of the handle it is called on, so the new space is nested where it was added. Its handler creates the space in p2panda and writes the space's first op, `POST [space]/origin`, carrying its name, services and manage list.
- `manage` defaults to the calling identity. An empty list means no one may delete the space or add members.
- Anyone may create a space. Attaching it to another space, so that it is visible to and managed by that space's members, needs that space to accept a `POST [space]/ref` first.
- The creator is a member.
- The set of services is fixed at creation; their versions still evolve. Adding services to an existing space is in the [backlog](backlog/add-services-to-space.md).

## Acceptance

- `GET [@carbon]/status` shows the new space's name and services, and the space accepts [chat] calls.
- A call to a service the space does not run is rejected.
