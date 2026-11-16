# A person can create a shared channel naming its services

Status: implemented — single-node; creating each service's space in p2panda comes with replication

## Calls

```js
const {slug} = await personal.call('POST [channel]/channel', {name: "Randall's Playhouse"})
slug   // 'randalls-playhouse'
const rph = personal.ref(`channel:${slug}`)
await rph.call('POST [channel]/services', {name: 'chat'})
// {name: 'chat', service, version}
```

## Rules

- A channel a person creates for themselves is nested in their personal channel, so its ref stays private.
- `POST [channel]/channel` runs on the handle of the channel it nests in and needs `manage` there, as every `[channel]` POST does. It takes a `name` shared with members and an optional `kind` (default `channel`), and derives a local slug from the name, adding `-N` with an incrementing integer when the slug is taken among refs of that kind; an optional `slug` overrides it. It answers the new channel's ref. `GET [channel]/refs` on the personal channel lists them.
- Its handler creates the new channel's space, running `[channel]`, and writes its first op there, `POST [channel]/origin`, carrying its kind, name and manage list; both are effects, made once on the op's first run and recorded (see [docs/replication.md](../../docs/replication.md#effects-are-made-once)). It then writes the ref in the calling channel. The origin repeats the create call's fields because the channel's members may not be able to read the channel it was created from.
- `manage` defaults to the calling identity. An empty list means no one may change the channel: add services or members, or delete it.
- `POST [channel]/services` names a service in the channel: `[name]`, run by `package`, a package registered on this device, which defaults to the name. Its handler creates the service's space as an effect and records the entry: name, service id, package and version. A name the channel already uses, or `channel`, is refused with a 409. A manager may add services at any time; removing one, and moving one to a new version ([backlog](../backlog/upgrade-package-version.md)), are not yet defined.
- A service belongs to one channel. The channel's members are the members of each of its services, at the same access.
- Attaching an existing channel to another, so that it is visible from there, is `POST [channel]/ref` on that other channel, which needs `manage` there. A ref grants nothing.
- The creator is a member.

## Acceptance

- `GET [@carbon]/status` with `{channel: rph.channel}` shows the new channel's name and services, and the channel accepts [chat] calls.
- A call to a service the channel does not name is rejected with a 404.
