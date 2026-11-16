    1. When an op is refused after calling another service that wrote, do that service's writes stay or roll back?

I think it's fine if they stay.  The business logic of the service itself can be responsible for consistency: it should know what to do if a subservice call fails.

    2. How does an op stay applied exactly once across a crash, when its writes and its result live in different files?

I think your proposal answers this.

    3. One file per service per space, or one per sql binding?

Per SQL binding.

Even though this answers some questions for us and simplifies things, allows us to eliminate some code...  I'm concerned that it also may reduce the meaning of "spaces" as an abstraction to build on.

Current Model:

- 1 oplog per space
- a set of services selected for the space
- state is kept consistent per space, which makes sense: state management has to happen at the same granularity as the oplog
- [pull, read, write, manage] managed per space

If we go to Per Service DB:

- Also probably want a space per service (oplog)
- Need some sort of bridge semantics for establishing communication between them
- Need [pull, read, write, manage] permissions per service
- putting services into a space was a natural way of establishing a "team" boundary, now we would need some other abstraction for that

I think this probably hinges around "bridge semantics" between spaces, and whether that's something well supported in p2panda.

---

    Consistency per space. Your point that state is managed at the granularity of the op log still holds. That granularity is the space; files sit beneath it.

Mmm...  many service calls will have subcalls.  If you have a subcall tree:

  A
  |_ B
  |_ C
     |_ D

We store that as a single atomic op with subcalls.  It gets replicated that way.  If the parent op is in a @local space, then only the subcalls get replayed.  (This logic can probably be eliminated by the way once we have proper WASM boundary side effect semantics.  Set that aside as another topic, off critical path.)

My concern is that if you rewind A and play it back, now you have to rewind B, C, and D as well.  Which could be on different services.  So now you're coordinating rewinds of database state because your op granularity doesn't match your database ACID granularity.  That was the whole reason why we consolidated the services in a space into one database to begin with.

    On bridges in p2panda: there's nothing to build on. Each p2panda space is its own group with its own stream. There's no shared order, no atomicity and no bridge between spaces. A space per service would mean inventing all of that ourselves, which is the strongest reason to keep the space as the unit.

Heard.  But bear with me for a moment: Is there a JOIN logic by which spaces can be composed?  Should there be?  If we had a mathematically consistent way of doing so, implemented at p2panda layer, would that allow us to work from more granular building blocks?

---

    I added a topic off the critical path: "Local ops replay through side-effect semantics, not their own path",

Rename that to: "Refactor: local ops can run replicated once side-effect semantics are complete"

    1. Files per binding inside one space: yes, rewinds would span files.

This is the reason I stated earlier for ops and DB ACID granularity to match.  I think you prematurely discarded that, and I think you can see that now.

    3. Composing spaces as actors does give granularity.
    - A call from a service in space X to a service in space Y becomes an op posted into Y's log.

Good.  You have my attention.

    - On X's side it's a side effect: made on the first run only, with replays getting the recorded response. That's the default policy we're settling in the side-effects topic.

Yes, so the side effect distinction continues to be important.

    - Each space applies its own log, converges, and rewinds on its own, and never rewinds another space.

Yes.  And: space = actor = service

    - p2panda needs nothing new, except that the calling device has write access to Y.
    - The price is eventual consistency between spaces, the same as between Durable Objects.

But what means is an eventual consistency guarantee provided?  It's provided for a replicated service.  But I don't see where any cross service gaurantee would come from.

---

    Nothing guarantees agreement between two spaces today. Only delivery is guaranteed.

That's fine, and I think we should stop there.  That means that we've created actor model semantics in a p2p setting.  And that's sufficient.  People know how to build apps on actor model, and it's a sane and reasonable way of doing so.

    What doesn't hold: agreement on the answer.

Isn't that fine?  Doesn't it maintain the same semantics as web programming?  And furthermore, it frees up services to have their own independent hosting and data custody decisions.  We build the interconnectivity between them based on communication, without the expectation of them being collocated.

    Where a guarantee could come from: report back.

Doesn't that violate the internal state isolation of an actor?  Actors receive messages, and send messages.  They're not responsible for transcribing changes to their internal state.

---

    An answer from a replicated space is that replica's answer at that moment, and a late op can still change it.

Can we offer the ability to wait for a quorum?  Since we're considering service-to-service calls as a side effect to be provided through the WASM boundary, the quorum setting can be part of the side effect binding configuration.  Does p2panda provide facilities to make this possible?

    How does a message reach a space the caller isn't a member of?

p2panda has [pull, read, write, manage].  `write` is sufficient to message a space, yes?

In addition, completely out of band to what we are discussing here, a service may bind its routes to HTTP and respond to RBAC and cryptographic identity.  (We don't need a topic for this in the agenda, just discuss.)  That is another form of inter-service communication, which doesn't even assume they are on the same network topology or that they are all hosted by CarbonDB.  It's not different in nature from any other HTTP call.  Mediated through the side effect binding.

    When the calling device does hold a replica of the target, does the call get that replica's answer at once?

I would say yes, and in most cases this is how the application development model will probably work.  With the caveat that a quorum setting on the binding might be useful in cases where the application developer knows they want that.

Ok... go ahead and open a new agenda to focus specifically on this possibility.  Document the topics related to:

1) Prefixed with "DROP:" What parts of the system will go away
2) Prefixed with "ADD:" What new components need to be created
3) Prefixed with "DOCUMENT:" consequences we need to understand and document
4) any other open questions

---

    In p2panda, each access level includes all the lower ones (p2panda-auth/src/access.rs:13-15), so write includes read

Oh.  Does read also include pull?  IE: any level of access makes this device a replication peer?

    The question is: does "space = actor = service" mean the service is the code

Claude, the notion of service is well defined, and you are swimming in examples of them.  The ../otto directory is full of one form of them.  ../microql and ../microql-services has another form.  CarbonDB has its own form.  These three forms are different, but ideally compatible?  But you shouldn't be confused about this at this stage in the game.

One service per chat room: We might run one instance of [chat] per company.  Or we might run one for the marketing team.  Or one for our friends "Soccer in the park" group.  The question of whether these are separate chat services is a question of data custody, and varies based on the use case.

In general CarbonDB represents a more granular way of managing data custody, rather than "put all your data in the cloud and hope for the best".

I don't know if that answers your question.

---

    A space is one instance of one service

I would say if we go with this notion of: service = space = actor

Then we should stop talking about "space" as its own domain term, and it's simply an implementation detail of what a "service" becomes in CarbonDB's ecosystem.

    write-only access for messaging doesn't exist in p2panda as it stands

That's fine.  I think that will be implemented by HTTP: you can message the system, but you're not a replication peer.

Hmm... the critical thing I want to discuss here is whether we should perform this rewrite at all.  Can you spell out pros and cons that we know at this point?  What factors might swing the balance?

---

    The biggest risk is not the code but membership: a team that uses five service instances means five sets of members to keep in step.

Right, but again this is a data custody question.  You might want to give John replication access to [chat] and [worklog] but only RBAC mediated REST access to [books].  The service level granularity is actually stronger here.

    4. The built-in services are coupled. [device] calls [space] and [space] calls [identity], all in one identity space today. They'd need redesigning, or to become the core's own records rather than services

That's not just an artifact of the builtins, it's fundamental to the design.  It's fundamental to the mental model that we're offering our developers.  I haven't seen you devote a lot of energy on this topic, and to me it is the most important deciding factor.

We need to actually re-implement these semantics in this model.  It is fundamental to the Otto model, and to MicroQL as well.  You have a catalog of services referred to by "pet" names.  They exist in the same namespace.

In Otto that's implemented by candle-connect, and by CORS settings, by the [identity] service access controls.

In MicroQL it's implemented by the registered services that are callable from within a query.

In CarbonDB, it was implemented up til now by the "space".  And if that goes away, we need to rebuild it.

So the primary question I have to you is:  What does it look to rebuild the namespace by which services know about each other and communicate with each other, if that is not to be done via a "space"?  And is that "worth it"?  Or are we giving up much of the benefit that p2panda provides?

---

    whose directory does a call resolve through?

Let's call it a "channel" not a directory.  The channel should itself be a space.  Replication of the "channel" distributes it to peers, and then each of those peers read the ops and:

- download service WASMs (if required)
- run service WASMs in [pull, read, write, manage] mode

identity:Brandon
  |_ device:Pixel9 [manage]
  |_ device:MacMini [manage]
  |_ device:CloudflareRelay [pull]
  |_ ref:FooCorp
channel:FooCorp
  |_ [chat]
  |_ [worklog]
  |_ [books]

IE: the channel is a signaling layer that directs further deployment of software and data.  Device policy sets the maximum responsibility that a device is willing to receive.  In this way users can configure blind relays.

To answer the original question:

    whose channel does a call resolve through?

Service membership should always be a subset, or even equal to channel membership.  Therefore all peers possess the same channel.  Does that answer your question?

---

    Since every peer of a service also holds its channel, a call to [name] always resolves through that channel, and every peer resolves it the same way

Yes.  The channel will hold the mapping of [name] -> space.

The same way in Otto candle-connect holds [name] -> host.

We're just resolving to p2p instead of http.

    - The channel is a space whose ops name services by pet name. Replicating it tells each peer which WASM components to fetch and what to run, so software and data deployment follow from the channel.

Maybe.

1) the channel itself could probably be implemented as a service, complete with SQLite persistence.
2) it may contain other signaling/negotiation data, such as being invited and accepting/rejecting

How does this intersect with our current concept of @carbon and space services?

    Three questions this leaves:
    1. What does a channel entry hold?

Maybe not as uniform/single op as you are assuming.

    2. Can one service instance appear in several channels? For example, Brandon's [identity] is named in his personal channel and in FooCorp's. If so, which channel does a call resolve through: the caller's own?

Mmm... I want to say no.  And you're wrong: [identity] (a service) is not named by other channels.  And "identity" the space (in current implementation) is also not being passed around or granted access from communication channels the person is in.  The cryptographic proofs associated with the person may be transmitted, and may be used to grant access and authorizations.  The "identity" space is shared between that person's devices.  And the [identity] service runs in that space to mediate access.  Let's not muddy the waters by discussing this further as long as what I've stated is clear.

    3. What access does a device end up with? My proposal: the lower of what its identity is granted on that service and what its device policy accepts.

Yes.  I think that was already clear from what I said before.

Mmm... can you identify the major design decisions at this point, from which everything else will flow?  And propose designs for them?  I will review what you write and decide whether to accept them into the agenda.  Please present this in a table, and try to be concise.  I will ask if I need further context and explainations.

---

    #: 1
    Decision: What a service instance is
    Proposal: One p2panda space holding one service, in one SQLite file with its op log and results. One sql binding per service, so an op and its transaction always match.

Yes.  Implicit.  You didn't have to ask.

    ────────────────────────────────────────
    #: 2
    Decision: What a channel is
    Proposal: A built-in service, [channel], running in its own space with SQLite. It holds [name] → service space, which version runs, and invitations and their answers.

Ok... there's more here that bears exploring...

    ────────────────────────────────────────
    #: 3
    Decision: How services call each other
    Proposal: [name] resolves through the caller's channel. The call goes through the side-effect binding: a local replica answers at once and is recorded;

Can a space ID resolve as some kind of string or serializeable value that can be transmitted itself in the channel space, mapped to the service name?

    otherwise HTTP.

This occurs via explicit mapping.  You would never take a p2p protocol service and fall back to HTTP.

    ────────────────────────────────────────
    #: 4
    Decision: Who deploys what
    Proposal: [@carbon], on each device, reads the channels its identity belongs to. It joins service spaces and fetches their WASM by hash, at the lower of the identity's grant and the device's policy.

Don't use the [@carbon] nomenclature when you're not talking about calls to the [@carbon] service.

What you really mean is that CarbonDB core is responsible for doing this.  It's part of the init/maintenance loop.

    ────────────────────────────────────────
    #: 5
    Decision: Where [space]'s records go
    Proposal: Membership and manage: p2panda, plus the core's own records in each space's file. Origin: the core's records too. Refs: the channel's entries, and the identity's refs to its channels.

I think maybe [space] becomes [channel].  What it was before (a collection of services that can talk to each other) is now accomplished by a channel.

    ────────────────────────────────────────
    #: 6
    Decision: Identity
    Proposal: Unchanged: a space shared by the person's own devices, running [identity] and [device]. Channels never name it; proofs travel to grant access.

Yes.

    ────────────────────────────────────────
    #: 7
    Decision: SQL inside a service
    Proposal: No namespace rewrite. The authorizer records reads and refuses ATTACH and pragmas that reach another file. sqlparser is kept only to narrow reads to one row.

Yes.  We already agreed upon this, it was fundamental to our early discussions.

    ────────────────────────────────────────
    #: 8
    Decision: Domain terms
    Proposal: identity, channel, service. "space" becomes an implementation word.

Yes, again, already agreed.

    What follows from them:
    - Rewind stays within one service space (from 1).
    - Stories 2 and 7, enrolling a device and adding members, are rewritten around channels (from 2, 4 and 6).
    - The side-effects design becomes a prerequisite for calls between services (from 3).

Yes.

What remains unclear?

---

    Still unclear, on the critical path:
    1. What a channel holds beyond [name] → space id

We need to unpack examples and discuss this on its own.

    2. Where a service space keeps what [space] used to keep for it

Uh... I don't think the builtins make as much sense as they used to.  Yes, the functionality needs to be provided in some way.  But does it need to be a replicated service?  This needs to be revisited.  You could probably let core throw some metadata ops onto each space, in accordance with the type and purpose of the space.

In some ways I liked the old model, the fact that we could have a tight focused core, and some things like querying the space, finding refs, could actually be exposed as callable by the services running within that space.

    3. Calling a p2p service this device can't run

Yes, unreachable.

Umm...  I've checked out a feature branch "channel".  Can you propose a rewrite to stories/services.md?  Call it stories/channels.md, but you should retain the same granularity:  I want to know how all of the existing service calls would be translated into the new paradigm.

---

1) You're still using the "space" language heavily, which we agreed to drop in favor of "channel"
2) Your preamble contained a lot of innappropriate language for the type of document this is
3) Does [device] still deserve to be its own service, or should it be joined with [identity]?
4) You have the old form (e.g. POST [@carbon]/space) as the primary mode of organization.  I want you to flip this around: organize everything in terms of the new form.

---

    On [device]: one service per space settles it. Merge device keys into [identity], and keep [device] only for the private part.

    - Device names and tags can't go into [identity]. An identity holds only what may be published.

Just leave the separation then.  I see there was a valid reason for it.

---

  - one beside the identity, holding device keys (POST [device]/origin, GET [device]/devices);
  - one in the personal channel, holding names and tags (POST [device]/ref, GET [device]/refs).

No.  I didn't agree to this.  Please stop and talk to me before making any further changes.  We should not be duplicating service instance which are intended to be connected to the very same state.

---

    My proposal:
    - it lives privately, shared only by the person's devices;

Yes, that's how it already works, and should be maintained in that way.

Go ahead and update the md.

---

Some suggestions:

1) use [channel]/origin; keep the origin pattern
2) what if we reconsolidate [identity] and [device] but we drop the intention of publishing identity's oplog as the public record of device authorization/reputation for that id?  We can defer the public record for now: it's not something we are presently designing around.  Does that solve the problem?
3) "device" still has some private key needs but that can be mediated by @carbon, as it currently is.
4) what can move out of @carbon once we have an implementation of side effects with proper semantics?

---

    without a published log, an invitation must carry the invitee's device keys itself

No it doesn't.  That can be part of the intial handshake.

    4. What could move out of [@carbon]

Debatable whether @carbon needs to still be explicitly a local service.  In practice it is local.  But it doesn't need to be from a perspective of calling side effects any more.  The more practical use of @carbon is to be something that operates in the system channel, before any identities exist.  As you say, most of the operations that occur nested inside identities and channels would no longer need to be done through @carbon calls.

So I would ask as well: can we drop @carbon from being something we include in all other channels?  Is it just a system thing?

---

Would it be bad to drop [@carbon] entirely?  Just use the carbon handle?  carbon.identities.add()?  And if we at some point want a "meta service" which has access to this kind of functionality, we provide it through the normal WASM boundary as a side effect?

Also, I think that given how heavily we are relying on side effects, we need an implementation of this registry in Rust code.  Then we can utilize it within our present services, and it should be a smaller matter to port them to WASM once we implement that.  This should be a new topic.

---

Maybe I shot myself in the foot here by using the /plan command, but the format in which you have typed services is really not what I had intended.  I want this to read as a manual for people interacting with CarbonDB.  You have put a lot of implementation details within it which makes it very hard for me to read and understand: what is the interface boundary that we are committing to?  Can you reformat it with that in mind?

---

    Should each call's answer be shown?

Sure.

I don't need an old to new mapping direct in the doc.  I think that might be better kept in the agenda, in ONE TOPIC, not scattered out as a bunch.

---

Ok, you still have:

    Called on: A channel or a service

Which refers to @carbon running in multiple spaces... which isn't a thing any more.

Instead of Input/Answer use Input/Output.

---

    Called on an identity's handle, by its own devices.

Hmm.  identity is a service that is... dangling outside of any channel.  Would there ever be a reason for other services to run as siblings to it?  Should it run inside the "personal" channel?

We decided on the current hierarchy for a reason:

- you need an identity first, and the cryptographic primitives it implies
- then you can enroll a device
- then you can create a personal channel
- the personal channel can later be shared, in case of caretaker, power of attorney, etc.
- but you wouldn't want to share the identity itself

What if identity itself was a channel?

system
 |_ identity:Brandon
     {ref -> FooCorp}
     |_ [identity]
     |_ [notes]
     |_ [vault]
 |_ channel:FooCorp
     |_ [identity]: Otto Service, RBAC
     |_ [chat]
     |_ [books]
     |_ [worklog]

carbon.ref('identity:Brandon').call('GET [vault]/passwords')
carbon.ref('identity:Brandon').ref('channel:FooCorp').call('GET [chat]/messages')

Or:

system
 |_ person:Brandon
     |_ [identity]

It's kind of weird right?  How do we sort out the relationship between:

- identity (cryptographic primitives and devices)
- identity (the service)
- personal

---

    With custody per service, you can share [notes] with a caretaker without ever sharing [identity]

Uh... I don't think you understand the fundamental nature of that proposal.  That's not how I want to control access for different identities.  It should be per channel, and should apply to all the services.

What if we say:

- identity is a channel
- it runs only the [identity] service (for now)
- we retain the "personal" channel, and this is where we default to nesting everything else under
- identity channel cannot be shared, nested under, etc: it is only for syncing amongst a user's own devices
- putting things in the personal channel therefore creates more flexibility, predictable behavior for the user

---

    Access is given per channel and applies to every service in it;

Let's say for now:  When a member is added to a channel with Access X, they are also added to all services with Access X.  But in the future we could change that behavior.  I just don't anticipate it being an easy concept for users/administrators to wrap their heads around.  So let's keep it simple for now.

    1. Your earlier example. You said John could replicate [chat] and [worklog] but reach [books] only through RBAC over HTTP.

Yeah, I'm mixing paradigms.  Let's set HTTP aside, it's out of band for our present design discussions.

    - (b) The identity channel also runs [channel], holding that single ref.

This feels cleaner to me in terms of API - I don't want to be duplicating.  But maybe we can just "burn" our write priveledges so that any further ops are refused.

---

    Taking away write and manage in p2panda would also stop you enrolling new devices.

Why?  Aren't those attached to the identity, not the [channel]?

    My proposal: a seal op. At founding, write the identity channel's POST [channel]/origin, then its ref to the personal channel, then POST [channel]/seal.

Yes, that works.

Write the plan.  Update the agenda.

---

    Is the invitation replicated, so other managers see it's pending and can cancel it?

Yes.

    Must a manager's device be online at that moment?

I hope not.  I think that the bearer of a cryptographic proof (which was sent with the invite) can be validated by all peers.  Right?

    The phone has to be added to FooCorp and to each of its services
    Unclear: Who adds it? p2panda needs manage to add a device, and John only has write

That's a good question.  Let's say someone with manage needs to be online and is presented with the request.  But that raises other questions in terms of multiple responders, etc... Let's put this aside in stories/channels/backlog/

    Alice upgrades [chat]

Backlog.

    John leaves, or Alice removes him

I think you can figure that out.

---

    actually adding the invitee in p2panda still takes a device with manage.

This speaks to us probably misusing p2panda, right?  Is this a new problem with the `channel` approach, or did it exist before when services were grouped in a `space`?

    Alice sends a bearer token

Approved.

    whether p2panda lets a member remove themselves without manage when leaving. If it doesn't, POST [channel]/leave stops this person's devices at once

I'm fine with best effort on this, and then report back what you end up implementing.

---

    That makes the upstream ask more valuable: once group writes work, the member-devices backlog story and the invitation key could both shrink considerably.

Can you transfer it over to the channels backlog and update it with the additional information?

Then lets discuss the last agenda item on critical path.

---


