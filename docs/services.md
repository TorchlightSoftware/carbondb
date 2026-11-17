# Channels and services

The goal is to make setting up a project planning board, a CRM, or bookkeeping, as easy as starting a group chat.

To accomplish this, CarbonDB runs services, which implement an application's functionality. Services live together in channels, where they can call each other, and any member of a channel can call its services. How channels nest, reach each other and grant access is in [channels.md](channels.md); how a service starts and what a call waits for, in [lifecycle.md](lifecycle.md).

## Concepts

- **Identity**: a person, held privately by their own devices. It is a channel of its own, running only `[identity]`, and points to one other channel, the person's **personal channel**; it is never shared, and nothing else nests under it.
- **Personal channel**: where everything else of the person's nests by default. It can be shared, for a caretaker or a power of attorney.
- **Device**: one of a person's devices, acting for their identity.
- **Channel**: a group of services, each named by a pet name such as `[chat]`, and the people given access to them.
- **Service**: a replicated app, complete with an API, a UI and SQLite storage.
- **Access**: an invitation to a channel carries `pull`, `read`, `write` or `manage` access, and a member has that access to every service in the channel. An identity can also set the most access each of its devices accepts.

## Making calls

A handle acts on a specific channel through a specific identity. `carbon` is the root system; an identity and its channels are reached through refs:

```js
const foocorp = carbon.ref('identity:alice').ref('personal:alice').ref('channel:foocorp')
await foocorp.call('POST [chat]/messages', {text: 'hi'})
```

A call on a handle reaches the service the channel names `[name]`; a name the channel does not name gets a 404. A GET is answered from this device's copy and never becomes an op. A write runs in one transaction in its service's own file: its queries answer at once, and its writes commit when the handler resolves or roll back when it fails. A call it makes to another service in the channel is a message, made once and recorded; what that service did stays even when this call then fails ([replication.md](replication.md#effects-are-made-once)). A write that fails on this device is never published.

### Watching

```js
const stop = foocorp.watch('GET [chat]/messages', render)
```

- An app renders tables through GETs, never ops. `watch` runs the GET, then again whenever a committed op writes a table the GET read. CarbonDB knows what a GET reads (SQLite's authorizer) and what an op writes (its session changeset), so any GET can be watched.
- A write's caller hears back once the watches its commit owes a run have run again, so a watched view already shows the write.
- When the GET fails, the listener receives `undefined` and the error, and the watch runs again after the next commit that writes what it read.
- Identity-level changes are GETs too, such as `GET [identity]/devices` on the identity's own channel; there is no separate event API.
- Not built: watches on the system handle, such as `GET [@carbon]/identities`; narrowing a watch from tables to rows; and re-running a watch once for a batch of synced ops, which needs replication.

## [@carbon]

A local builtin service that manages cryptographic identities and system calls.

| Call | Input | Output |
|---|---|---|
| `POST [@carbon]/identity` | `slug`, `device` (this device's name), `services` (the personal channel's), `code` (to join an existing identity) | `{slug, id, channel, personal, device}`: the identity, its own and personal channels, and this device's public key. Without a `code`, founds the identity and its personal channel; with one, joins it once a device already in it accepts, which is not built yet (`501`) |
| `POST [@carbon]/devices` | | `{code, device}`: a key pair for this device to join an identity with, and the code that shows it, for a device already in the identity to scan with `POST [identity]/devices`. This device then joins the identity once that device sends it ([docs/sync.md](sync.md#enrolling-a-device)); single-node, `501 SINGLE_NODE` |
| `GET [@carbon]/identities` | | `[{slug, id, channel, personal, device}]`: the identities on this device |
| `GET [@carbon]/status` | `service` (optional) | `{name, access, sync, peers, state}` for a service, and `error` once stopped; for the channel, its own `state` and also `services: [{name, access, sync, state}]`. `state` is `initializing`, `idle`, `applying` or `stopped`. `access` is the acting identity's; `sync` is `off`, `waiting` or `on`; `peers` are the devices this one has synced with, and when |
| `POST [@carbon]/sync/start`, `/sync/stop` | `service` (optional) | `{sync}`. For the channel, starts or stops it and each of its services this device runs |
| `DELETE [@carbon]/data` | `service` (optional) | `{}`. Deletes this device's copy of the service, or of the channel with its services |

## [identity]

A replicated service shared between all the devices that belong to an identity.

| Call | Input | Output |
|---|---|---|
| `GET [identity]/origin` | | `{id, device}`: the identity's id and its first device's public key, written when it was founded |
| `POST [identity]/devices` | `code` (optional), `name`, `tags` | Without a `code`: `{code}` for a new device to scan. With the `code` the new device shows: `{pubkey, name, tags}`, the device now acting for the identity |
| `PUT [identity]/devices/:pubkey` | `name`, `tags` | `{pubkey, name, tags}` |
| `GET [identity]/devices` | | `[{pubkey, name, tags}]` |

## [channel]

A channel mediates access to its services. `[channel]` is itself a replicated service, running in every channel, offering the controls and information that belong to that channel.

| Call | Input | Output |
|---|---|---|
| `GET [channel]/origin` | | `{kind, name, manage}`: what the channel is, written when it was created |
| `POST [channel]/channel` | `kind`, `name`, `manage` (default: you), `cascade` | `{channel, kind, name}`: a new channel inside this one, which this one points to |
| `POST [channel]/seal` | | `{sealed: true}`: the channel refuses every later POST |
| `POST [channel]/services` | `name`, `package`, `manage` | `{name, service, version}`: a new service running the package, named `[name]` in this channel |
| `GET [channel]/services` | | `[{name, service, version}]` |
| `POST [channel]/members` | `contact`, `access` | `{invitation, code}`: an invitation for the contact to accept with `POST [channel]/join`. The code carries the proof every member's device checks the acceptance against |
| `GET [channel]/invitations` | | `[{invitation, contact, access, by}]`: invitations not yet accepted, visible to every member |
| `DELETE [channel]/invitations/:invitation` | | `{}`: cancels an invitation; its code no longer joins |
| `GET [channel]/members` | | `[{identity, access}]` |
| `DELETE [channel]/members/:identity` | | `{}`: removes a member from the channel and every service in it. What they wrote before stays; what they write after is refused |
| `POST [channel]/leave` | | `{}`: you leave the channel and every service in it, and your devices delete their copies. Needs no `manage` |
| `POST [channel]/join` | `code` | `{channel, kind, name, slug}`: joins the channel an invitation's code names, and points to it from this one |
| `POST [channel]/manage` | `add` | The channel's manage list |
| `GET [channel]/manage` | | The channel's manage list |
| `POST [channel]/ref` | `channel`, `kind`, `name`, `cascade` | `{channel, kind, name, slug}`: a pointer to another channel. It grants nothing |
| `GET [channel]/refs` | | `[{channel, kind, name, slug}]` |
| `GET [channel]/ref` | `kind`, `slug` | `{channel, kind, name, slug}`, or a 404 |

`cascade` takes `remove`, where losing access to this channel removes access to the one it points to, and `delete`, where deleting this channel deletes that one, given `manage` there.

## Not yet defined

- A member's new device joining their channels ([member devices](../stories/backlog/member-devices.md)).
- Upgrading a service to a new version ([upgrade package version](../stories/backlog/upgrade-package-version.md)).
- Where a person sets a device's policy, the most access it accepts.
- Deleting a channel or a service, and revoking a device.
