# A person can create a shared space naming its services

Status: agreed

## Calls

```js
const rph = await identity.createSpace({name: "Randall's Playhouse", services: ['chat']})
rph.slug   // 'randalls-playhouse'
```

## Rules

- Creating a space is part of the `carbon` interface; it publishes the space's first op, `POST [carbon]/space`, carrying its name and services.
- The creator is a member.
- The set of services is fixed at creation; their versions still evolve. Adding services to an existing space waits for a concrete use case.

## Acceptance

- The new space lists its name and services, and accepts [chat] calls.
- A call to a service the space does not run is rejected.
