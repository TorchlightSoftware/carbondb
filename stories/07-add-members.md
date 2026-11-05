# A person can add members to a shared space

Status: agreed

## Calls

```js
// Alice has Sally's contact, exchanged by QR code
const {code} = await rph.call('POST [@carbon]/members', {contact: sallyContact})

// on Sally's device, once the code reaches her by any medium; the ref lands in her personal space
const rph = await sallyPersonal.call('POST [@carbon]/join', {code})
```

## Rules

- A space is a message bus; adding a member adds one more participant. The space is implicit in the handle. The call is local and returns an invite code.
- A contact holds the identity's p2panda group id and how to reach one of its devices. An invite code holds the space id and how to reach the inviting device. What carries either is purposely undefined; for now, a QR code or a copied code.
- Joining subscribes to the space, which announces the device's key bundle. A device with manage then adds the member in p2panda and writes `POST [space]/members`. That device must come online at some point after she joins; until then her access stays empty.
- Members get `write` unless the call passes another access level.
- Joining writes a `POST [space]/ref` in the space it is called from, unless that space already holds one, so the joining device needs the `manage` tag. Reaching a space through refs needs both the refs to exist and access to the space, and what is possible there depends on that access.
- Membership takes effect only when the invited identity joins on one of its devices; nothing is sent to her before then.
- Links, directories or a relay registry may come later.
- Membership decides access only; when a device syncs a space is set by `[sync]` in the TOML file. What a member may call is decided by roles in the app's own service, kept current by triggers on `POST [space]/members`.

## Acceptance

- After Sally joins and a device with manage adds her, her device holds the space and sees Alice's earlier [chat] messages.
- Before she joins, her device holds nothing from the space.
- After she joins and before she is added, `GET [@carbon]/status` shows `access: []` or `undefined`, and she cannot read the space.
