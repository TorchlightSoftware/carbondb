# Channels and services

The goal is to make setting up a project planning board, a CRM, or bookkeeping, as easy as starting a group chat.

In order to accomplish this: CarbonDB's runs "services" which implement application functionality.  Services exist in "channels" together where they can communicate with each other.  Services can be accessed by any members in the channel.

## Concepts

- **Identity**: a person, held privately by their own devices. It is a channel of its own, running only `[identity]`, and points to one other channel, the person's **personal channel**; it is never shared, and nothing else nests under it.
- **Personal channel**: where everything else of the person's nests by default. It can be shared, for a caretaker or a power of attorney.
- **Device**: one of a person's devices, acting for their identity.
- **Channel**: a group of services, each named by a pet name such as `[chat]`, and the people given access to them.
- **Service**: A replicated app complete with and API, UI, and SQLite data storage.
- **Access**: An invite to a channel will contain `pull`, `read`, `write` or `manage` access, and a member has that access to every service in the channel.  An identity can also set the maximum priveledge level of each device within it.

## Making calls

A handle acts on a specific channel through a specific identity. `carbon` is the root system; an identity and its channels are reached through refs:

```js
const foocorp = carbon.ref('identity:alice').ref('personal:alice').ref('channel:foocorp')
await foocorp.call('POST [chat]/messages', {text: 'hi'})
```

## [@carbon]

A local builtin service that manages cryptographic identities and system calls.

| Call | Input | Output |
|---|---|---|
| `POST [@carbon]/identity` | `slug`, `device` (this device's name), `services` (the personal channel's), `code` (to join an existing identity) | `{slug, id, channel, personal, device}`: the identity, its own and personal channels, and this device's public key. Without a `code`, founds the identity and its personal channel; with one, joins it once a device already in it accepts |
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

Channels create shared spaces that mediate access to services.  [channel] itself is a replicated service which offers controls and information pertaining to the channel it is hosted in.

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

- A member's new device joining their channels ([member devices](backlog/member-devices.md)).
- Upgrading a service to a new version ([upgrade package version](../backlog/upgrade-package-version.md)).
- Where a person sets a device's policy, the most access it accepts.
- Deleting a channel or a service, and revoking a device.
