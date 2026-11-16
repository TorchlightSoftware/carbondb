# A person can add members to a shared channel

Status: agreed — needs peers until p2panda; single-node, these calls fail with `503 UNREACHABLE`

## Calls

```js
// Alice has Sally's contact, exchanged by QR code
const {invitation, code} = await rph.call('POST [channel]/members', {contact: sallyContact, access: 'write'})
await rph.call('GET [channel]/invitations')                        // pending, visible to every member
await rph.call(`DELETE [channel]/invitations/${invitation}`)       // or cancel it

// on Sally's device, once the code reaches her by any medium; the ref lands in her personal channel
const {slug} = await sallyPersonal.call('POST [channel]/join', {code})
const rph = sallyPersonal.ref(`channel:${slug}`)

// later
await rph.call('GET [channel]/members')
await rph.call(`DELETE [channel]/members/${identity}`)             // a manager removes a member
await rph.call('POST [channel]/leave')                             // a member leaves
```

## Rules

- A channel is a group of services and the people given access to them; adding a member gives them that access on every service in the channel. The channel is implicit in the handle. Each of these calls makes its p2panda change as an effect, once, on the device that made it.
- Access is `pull`, `read`, `write` or `manage`. Members get `write` unless the call passes another level. A device takes the lower of what its identity is granted and what its own policy accepts, so a pull-only device is a blind relay: it carries ciphertext it cannot read.
- An invitation works through an invitation key: the inviting manager adds the key as a member when inviting, and the code carries its secret as a bearer token. The bearer joins as that key at once, and a manager's device later replaces it with the invitee's own devices. Cancelling an invitation removes the key, so its code no longer joins.
- Invitations are replicated in the channel, so every member sees which are pending and a manager can cancel one. The code carries the proof every member's device checks an acceptance against, so validating it needs no manager online. It need not carry the invitee's device keys: they are exchanged in the initial handshake.
- What carries a contact or a code is purposely undefined; for now, a QR code or a copied code. What a contact holds is not yet defined.
- `POST [channel]/join` runs on the channel that will point to the joined one, by default the personal channel, and needs `manage` there. It joins the channel the code names and writes a ref to it. Reaching a channel through refs needs both the refs to exist and access to the channel, and what is possible there depends on that access.
- Membership takes effect only when the invited identity joins on one of its devices; nothing is sent to her before then. Which op records her as a member once she accepts, as `GET [channel]/members` lists her, is not yet defined.
- Adding the invitee's devices in p2panda takes a device with `manage` on the channel, the same limit as a member's new device ([member devices](backlog/member-devices.md)); each device joins every service's space in the channel ([upstream group write](backlog/upstream-group-write.md)).
- Removing a member takes them out of the channel and every service in it: what they wrote before stays, and what they write after is refused. Leaving needs no `manage`, and the leaver's devices delete their copies; it is best effort, taking the member out in p2panda if it allows that without `manage`, and otherwise stopping their devices at once and leaving the removal to a manager. The cutoff for a removed member's unseen work is in [remove member](../backlog/remove-member.md).
- Links, directories or a relay registry may come later.
- Membership decides access only; when a device syncs is set by `[sync]` in the TOML file. What a member may call is decided by roles in the app's own service; how that service learns of membership changes is not yet defined (triggers on `[space]` were dropped).

## Acceptance

- Before she joins, her device holds nothing from the channel.
- After Sally joins with the code, her device holds the channel's services and sees Alice's earlier [chat] messages, with no manager online; once a manager's device is online, the invitation key is replaced with her own devices.
- A cancelled invitation's code no longer joins.
- After she is removed, her later writes are refused on every replica and her earlier ones stay.
