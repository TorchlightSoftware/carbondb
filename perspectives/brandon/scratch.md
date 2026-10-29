# Further Comments

    "p2panda keeps your data on your devices and those of the people you share with"

This more closely matches what I'm looking for.  Who I share data with should happen by informed consent, not broadly by default.

    "the Rust core then has to run in a browser (WASM) or behind a desktop shell"

Yes.

    port River

How fully featured is it?  I don't want to port something massive, I want the simplest possible demo app to compare?

    add Veilid

Say more.  I'm looking at it for the first time.  Can you add an assessment for it in /research?

    Keyhive, from Ink & Switch

It appears they have applied a capabilities model to CRDTs.  I had the thought to do so years ago but the implementation details escaped me.  I think probably as a consequence of their approach:

1) auth and CRDT are baked in at a fundamental level
2) any other data types have to be described building up from those primitives

It's possible that this is necessary in order to get the kinds of benefits we want from the local-first, p2p paradigm.  But it's a heavy ask: it probably means abandoning candle-db and starting over.  It means that the cross runtime portability that Otto seems to have so far would hit a hard wall in that the fundamental data types are not CRDT, and therefore not compatible with the P2P paradigm.  Unless there's a transform that's possible.

Yes.  Add an assessment for this as well in /research.

    OpenMLS: We should compare p2panda's custom, unaudited group encryption against it

You can add that as a note in the p2panda research.

    Willow and its Meadowcap capabilities

Let's not.

---

    Even p2panda leaks your network details to iroh. Its relays and discovery servers still see IP addresses and timing.

Does Freenet use "relays and discovery servers"?  I think that's the rub: discovery has to happen somewhere.  It's either a ring architecture or it's a relay server.

    My proposed demo is command-line only, roughly a few hundred lines per platform (estimate):
    1. Alice creates a room: a signed, append-only message list.
    2. Alice invites Bob with a QR or link ticket, and Bob joins only by accepting (consent).
    3. Both post, including while offline, and the room merges when they reconnect.
    4. Alice enrolls a second device, which receives the history.
    5. Alice removes Bob, and Bob can't read new messages.

Yes.  That's much more appropriate than porting a 200kloc codebase.

I'm not interested in any further discussion of Veilid.  I've got limited bandwidth and it doesn't fit in my mental space right now.

    Keyhive doesn't require CRDT data; it requires your data to have a history

I built in an optional audit log which plays well with candle-db.  So maybe there could be a translation based on that.

    - Keyhive is not ready to depend on: It is pre-alpha with no code audit

Noted.  Still interesting to me as I greatly appreciate their approach.

    https://www.inkandswitch.com/keyhive/notebook/

    We believe that this choice is appropriate for static control context on documents that require the entire history.

I'm a little bit reticent to use a technology like this.  My experience with tech like "Secure Scuttlebutt" was waiting forever for it to download histories before I could see the chat.

But maybe this can be solved by proper segmentation...  Maybe the authors discuss solutions elsewhere...

    we use the Automerge Binary Format to compress-then-encrypt ranges of changes. We expect these encryption boundaries to change over time as parts of the document become more stable

This hints at maybe that is what they are doing... using chunking to give good tradeoff options.

    don’t make ciphertexts retrievable by anyone, but only those with “pull access” or higher

I like that they are explicitly thinking about this.

---

    Signed, single-writer, hash-chained logs

Does that mean that each "conversation" or "shared document" has a single owner, that owner is the writer and the source of truth?

1) Correct me if I'm wrong: p2panda does have a concept of a device constellation?  IE a member who operates from multiple devices?
2) Does this architecture mean that a single device controlled by that user is designated to be the source of truth?
3) What thought and accomodation has been put into availability?  E.g. if Bob wants to operate a chat room and Alice and Kim want to join, Alice and Kim should still be able to talk when Bob is offline on his mobile phone.  This necessitates some kind of server - not in that it has special priveledges, but in that it has always on, always connected gaurantees.  This could be a home server, or it could be a cloud server.
4) Ideally the "server" or "relay" should have limited permissions: in the KeyHive nomenclature it has "pull" permissions but not "read/write" permissions.
5) But in addition we might want the server to have the capacity to run contracts, so that Bob can say "these are the messages that are allowable" and that filtering could happen in a high availability context.  This necessitates some kind of boundary for what portions of the message are E2EE vs visible by the server.  Whatever data on which you are making decisions on must be visible to the server.
6) In order to make the servers as blind as possible, said filtering could be done in stages: deterministic logic is used to assign tags to messages.  The possible message tags are represented as an enum (encoded as integer based iteration, or a deterministic algorithm of choice).  The contract for "filter these messages" has its tag values replaced by the symbolic representations of the tag.  Therefore the server only has symbolic, anonymized references to the meta-data and contract-meta-data references, and otherwise only has encrypted data.  The symbolic references could be rotated per conversation, per session, per time window (or other appropriate scoping), making it more difficult to deanonymize them (figure out what metadata they represent).  I'm open to further guidance on how to do this, existing algorithms that are appropriate...
7) the relay server therefore, could make certain assumptions about the shape of data and associated tags, but would have no supporting evidence for the content
8) While the E2E nodes (with data visibility) are responsible for calculating these tags, and they could lie and have incorrectly formatted messages accepted by the relay, 1) the other nodes who can read the messages will detect and reject the messages, 2) the offending node's reputation will suffer, 3) at some point they can be kicked/banned from the channel.  So we have enforcement from peers without revealing plaintext to the relay.
9) It's possible that some cryptographic proof of bad behavior could be provided so that the server with proper evidence from members (still not revealing the content) could act to kick someone from the channel.  Bob being online is not required.
10) I think p2panda would be more flexible for this type of extension.  Do you agree?  Do you see any barriers to implementing something like this?

    Freenet enforces writes in the contract and has no read control on the network

Right.  Which I think in the relay model I've described above, means that the relay requires visibility into the underlying data in order to enforce said contract.

    A delegate holds keys on the device. It can ask the user for permission before signing, and the runtime tells it which app is asking

This is a useful abstraction.  Something like this needs to be implemented for sure.

    Pull < Read < Write < Manage, each narrowable by conditions such as a data path, and only Manage can change membership

That sounds incredibly helpful.  And close to the model described by KeyHive.

---

Folder changes: I renamed "research" to "platforms" and deleted "plan" and added "components".  What is currently in the agenda topics are the components.  Can you take them out of agenda, and create files in the components directory for them?  Then close the agenda.

---

Drop Veilid.

    Each device writes its own log, and a room is all of those logs merged

Ah, fair.

    There's no source of truth

I misspoke, and that makes more sense actually.

    a member can be a group of device keys, each with its own log
    QR enrollment and recovery are ours to build

Understood.  These are important notes to have in the p2panda research document.  And whatever parallels apply to freenet.

    p2panda has no always-on node, so we build one, and it can be Pull-only

Roger that.

    That lets a Pull-only relay check membership, and it leaks membership to anyone watching the network.  Their planned encrypted control messages would hide membership from the relay too

Totally acceptable for now.  And this is great.  Document where we can watch for updates on this: is there a github issue?

    A Freenet contract could check signatures and blind tags over ciphertext exactly as our relay would. So Freenet already has your items 3 and 5

Be sure to document that in the freenet research, and link to further information.

    With a small enum, how often each tag appears, plus timing, can reveal what it means

Yes.

    Mitigations: keep the enum coarse, pad it with unused values, add decoy tags

Even if you condense all tags into single bit fields set within an otherwise randomly generated integer (noise), the bit masks in the contracts would still reveal which data is actionable.

Other option: don't rely upon the server for any per-message processing.  Instead: offer a ZK proof "template" that must be met for a member's access to be revoked.  Therefore, any member may offer this proof and expect that it will be acted upon.  And even though ZK proofs are expensive, you only need to use it in the event of revocation, not in normal chat.

    in p2panda's per-message (Double Ratchet-style) scheme, each recipient receives a different ciphertext. "A sender can show Alice one message and Kim another, and the relay can't tell"

I'm not sure what you mean by that?  Are we not in a shared chat?  Are you speaking generically about the protocol in a way that chat semantics are not being enforced?  I believe that what I would consider "normal chat semantics" CAN be enforced in p2panda: that is we should all see the same thing, with some differences in ordering as an acceptable variance.

    Proving abuse without content means zero-knowledge proofs

Well... and what I am considering abuse is a schema violation, not a thought police violation.  I'm only considering metrics for which we have a deterministic and objective measure.

    the relay can cut a member's Pull immediately, but cutting their read access needs a key rotation by a member who can read

Fair.

I think we're getting close to being ready to build a demo here.  Can you propose an agenda that will accomplish the demo using p2panda, with topics covering the core build, and then covering the exploration of a blind relay server?

---

    p2panda already ships a 278-line encrypted group chat

That sounds great.  Can you clone that down and we'll take a look?


