# A person can add members to a shared space

Status: agreed

## Calls

```js
// Brandon has Sally's identity id from a contact exchanged by QR code
await rph.call('POST [carbon]/members', {identity: sallyId})

// on Sally's device
await sally.call('GET [carbon]/invitations')
const rph = await sally.acceptInvitation(invitation)
```

## Rules

- A space is a message bus; adding a member adds one more participant. The space is implicit in the handle.
- Membership takes effect only when the invited identity accepts on one of its devices; nothing is sent to her before then.
- Contacts are exchanged by QR code; links, directories or a relay registry may come later.
- Membership decides hosting only. What a member may call is decided by roles in the app's own service (in Otto, [identity]), kept current by listeners on `POST [carbon]/members`.

## Acceptance

- After Sally accepts, her device holds the space and sees Brandon's earlier [chat] messages.
- Before she accepts, her device holds nothing from the space.
