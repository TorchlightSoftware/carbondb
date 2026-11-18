Alright, review any planning documents related to p2panda.  We are going to implement p2p sync.  Create a new agenda to match.  Do you have any questions before we begin?

---

    A narrower goal would be two devices syncing one service, which is steps 1–3

Let's set that as critical path to begin with.

    Should I read perspectives/brandon/p2panda-scratch.md

That would be redundant.  It contains the messages I am typing to you now.

What questions do you have with this goal in mind?

---

What if you do:

- create identity on device 1
- enroll device 2 (same ID)
- add [chat] to personal
- chat between D1 and D2

This is a more logical starting point of what an actual user flow will be.

---

    1.
    Creating a service is an effect

I suppose you can look at it that way...  But I think you create a channel, and then load services into it.

    So device 2 needs the "ref" sync mode

I don't know what you're talking about.

    It needs that walk twice, once after enrolling and again when [chat] is added

Sounds inefficient, does it need to be?

    2. Adding [chat] after enrolling means adding a service must also put every existing device into the new p2panda space. That's the assignment policy reacting to a new service. The smallest default would be for every device of an identity to hold every service of its own channel and its personal channel, at manage.

Sensible.

    3.
    What's undefined is which call on the new device generates its key pair and the code.

I don't think that's undefined.  That's going to be a @carbon call.

    4. Your flow makes tying a received op's device to its author identity easy.

Ok.

Anything else?

---

    When device 2 applies the same op, it gets device 1's recorded result and builds nothing, so it never gets a local copy of [chat].

The way we defined our side effects model, we have the capacity to bind a replicated version.  So why don't you do that?

    The "ref" sync mode is in the docs,

You wrote the docs.  You sometimes write gibberish into the docs, and then repeat it back to me as though it were gospel.

    describes a loop that walks refs and joins services

That makes sense.

    makes ref the default mode

That does not.  Default mode for what?  What's the alternative?  What does that mean?

Oh... you're talking about a top level config on CarbonDB that changes whether it walks refs and loads all the spaces, or whether it lets them be manually loaded.

Ok.  Why are we talking about this?

    - Creating the p2panda space runs once, on the device that made the op, and its id is recorded as create's output.

Yes.

    - Opening it locally and joining the p2panda space runs on every device. That second effect would be declared replicate = "always", which is already built

Uh... no, you're confusing two different things:

- a setting that controls whether side effects run replicated, declared per side effect
- a setting that controls the behavior of the node, and whether it automatically loads newly referenced spaces, or lets the controlling app do that manually

The creation of the space is controlled by the former, and only runs once, but is observable by peers.

Whether a node loads that new space or not when it sees it is controlled by the latter.  It does not require that we "split today's create effect in two".

---

    how a ref-mode node notices a new ref or service without a polling loop

You already have channel.watch('GET [channel]/refs')

---

Simplify the goal to:

implement: create an identity on device 1, enroll device 2 in the same identity, add [chat] to the personal channel, and chat between the two devices.

---

    Which way does enrolling run?

I don't care.  Pick one.  You'll do it the other way when you're done.

    Does CarbonDB's service id become p2panda's space id, or stay derived from the op with p2panda's id recorded beside it?

There are two things which you need to track independently: the hash of the service, which determines what ops are valid for it to run, and the space id, which determines the particular instance of oplog.

    Does the p2panda node run on the core's single-threaded runtime, or on its own runtime that hands work to the core's thread?

I'm open to your advice here.

---

    On one LAN, a QR code plus mDNS is enough. Across networks you need a relay, though the QR code can still replace bootstrap.

Ok.  Make sure to document that, and we should test in various configurations.  Bootstrap still seems relevant - if it's configured then it's a well known place to post invites as well as identities and cryptographic claims.

I think your split between hard coded and config is good.  Let's do it.

---

    An iroh relay server helps open connections through NAT and forwards encrypted traffic. It holds no data and has no p2panda key,

Roger.

    CarbonDB's blind relay is a pull-only device of the identity

Yes.

    "add a relay device" could mean a pull-only device whose host also runs an iroh relay, giving the identity both at one address

Hmm.  I agree.  I think as well you could add iroh relays as part of the config.toml.  So they could come from either layer.

---

    Hold ops from unlisted devices

With a timeout, whatever you deem appropriate.

Everything else is accepted.

---

    24-hour timeout for unlisted devices

2 hour timeout, and I think that's generous

I also accept your proposals for this topic.

---

    The create effect now answers the space id

I have no idea what you're telling me.  You're speaking in backwards language like Yoda.  And the code in effects.rs is indecipherable: it doesn't read like a routing of known effects.  I'm sure you have that somewhere.

I think what you're saying is you offer access to create spaces through an effect... and I don't think that's what we agreed upon.  Creating spaces directly should only be done by core.  It's not a general purpose effect that any service could bind.

     - Membership adds: every device adds the listed devices missing from the spaces it follows, rather than only the device that made the op.

Maybe we do need a published version of the "identity" channel... read only so that peers can witness it.  Then they could have an up to date list of the identity's devices and add based on that.

    [@carbon] stays out of the 200ms timeout's way? No: founding an identity and showing a code still wait on node start inside that timeout.

Maybe we allow a default timeout per service.toml?

    the core test registers the scripted test service as [chat], because chat.wasm is gitignored.

What problem are you trying to solve?  Yes build artifacts are gitignored, and yes you can build them again if you need them... what does naming things have to do with build artifacts?

    Trust root for a new device

Sounds right.

    the mDNS case doesn't skip itself where multicast is unavailable

Can you fix that?

    identities in existing local databases, which have a bare key, won't start a node and must be founded again

Ok... do that.

    if an op that created a space is refused, or create fails after p2panda made the space, the p2panda space stays followed even though the local space is deleted.

I think we should fix that, but I'm not sure "create" was the right path to take, so let's talk about that first.

    If this pattern grows, a write-between-ops queue on the space itself would be cleaner

Or maybe just system ops, probably on the same oplog.

Also brings up the question of how @carbon or other local calls are handled visa vi the oplog... Perhaps they should be ops on the channel that they were called within.  This would create a pattern of "local and system calls ride on the channel/service they are called from".

    a dropped node is never explicitly shut down, because p2panda offers no call to do so. A restarted process is fine, but a same-process restart would run two nodes with the same id

Hmm?  Can you fix that?

Create a new agenda with these discussion points in it.  Goal: reviewing and cleaning up to prepare for commit.

---

    One table: a constant table of the known effects, each row giving the effect's name, its handler, and who may ask for it

Yes, but just put it in effects.rs

    should "who may ask" name exact packages

Only @carbon can do this.  And I don't think we have any other examples of effect restrictions for which that rule doesn't apply.

---

    [channel] asks for create when it makes a channel or a service

1) What is the effect actually doing?  Is it instantiating a ServiceChannel?
2) Before the "channel" rewrite we achieved "creating a sub-space" through a @carbon call.  If we define "local and system ops run on the current channel/service oplog" then we can do the same thing here, and the restriction to @carbon holds

---

    If a [@carbon] op logs on the channel's op log, its call into POST [channel]/services is the same kind of loop

Why is that a loop?  Show me.

    Question: do you agree with flipping the direction, so the app calls [@carbon] rather than [channel]?

Yes.

---

    a service that writes into itself through a call is refused as a loop

That breaks fundamental patterns that are used throughout any service architecture.  Services are meant to be used as a unit of composition, you can't just blow up when you see them nested.

1) for replicated sub-service calls, we nest them into the same parent op, right?  So they all run inside the same transaction.
2) for calls to another service, we send that as a side effect, it falls outside of the lock mechanism
3) for local/system, the op may be riding in the oplog, but it obviously doesn't lock the target service's database, so it shouldn't fall under the same lock rule
4) if a service ever calls @carbon, that would be done as a side effect

---

    An op that rides on the log without the lock needs its own path. It must be logged and have its result recorded without taking the turn

What if we put them in their own oplog?  It's a non-replicated oplog, doesn't need a space or any p2panda semantics.  But it's still nested under the [channel].

---

    Calls on the system handle, such as founding an identity or showing a code, aren't on any channel. My proposal is that they keep the system log, treating the system root as their channel.

Yeah, this reaffirms the need for a non-replicated oplog.

system
  |_ @carbon

Would both be instances of this.  And this would give you the ability to run other local services in a completely non-replicated mode if that's something that's desirable.  Maybe this makes the core more extensible.

Naming: We have ServiceSpace.  What if we also create LocalSpace?

---

root flag/local flag: Do these already turn off replication and not instantiate a space?  Maybe they are sufficient.

    1. Does a local op hold its LocalSpace's write transaction while its handler runs?

I like it because it maintains the same implicit transaction semantics.

    How does a package declare it runs as a local service?

I don't think that that's the decision of the service.  It's a consequence of how the service is registered on CarbonDB.  But maybe some services don't support replicated mode, which is an internal decision named in the service.toml.

---

    We agreed that a service calls [@carbon] through the call effect. But the core refuses that today with a 403, "a replicated service never calls a local one"

That doesn't apply any more since the implementation of effects.

    Since the call effect is made once and recorded, that reasoning no longer holds.

Yes.

    Should LocalSpace be a type of its own, or ServiceSpace with a replicated-or-local kind in place of root? I propose the kind, since nearly all the machinery is shared.

Sure, I see the logic in a flag.  But don't instantiate all the p2p/replication machinery for a local version.

Ok, is this clear now?  I feel like this is something I could have you implement now before we address the rest of the agenda.

---

    - Adding a channel or service

Agreed.  Your proposals are sensible.

    - Enrolling a scanned device

Can you propose a set of ops and show me the whole lifecycle of identity creation and device enrollment?

    - Creating a space: does it become a plain core step inside [@carbon], so that create leaves the effect table?

Yes.

---

I don't have a problem with any of what you've proposed.  I also wonder: could "system" be a channel that is simply local only?

    Questions

    1. Name for enrolling: POST [@carbon]/enroll on the identity's handle

Yes.

    2. Order inside enroll: I list device 2 in [identity] before adding it to the p2panda spaces. If the list write is refused, nothing else happens. If adding to the spaces then fails, the list entry stays, and the add-on-follow step repairs it. Is that the order you want?

Yes.

---

    Identities with a bare device key are founded again

I don't know what that means.

    Tell me the path, or say whether you'd rather delete them yourself

If this is a question about tests, I don't care where you put them as long as test isolation is ensured and the tests run deterministically and reliably.

    when a [@carbon] op fails, I propose it closes and stops following the p2panda spaces it created and removes their files

Ok.


---

    It's about system databases made by CarbonDB before this work, such as by the 0.1.0 release or an app you ran locally

Please stop trying to provide backwards compatibility.  That's not important at all.  You can nuke all that data and keep moving forward.

---

    1. A replica can't tell which service made an op, so [channel] can't refuse writes that didn't come from [@carbon].

The route can require a cryptographic signature from the device, which @carbon has access to do.  So this should be an agreement between the two services, not something that core enforces with special rules.  Does that make sense?

    My proposal: keep both tables in a file of their own per identity, beside its p2panda store, keyed by space. No op ever runs in that file, so every write lands at once and the queue goes away.

That's fine.

    DELETE [@carbon]/data

What does this mean?  This op is really unclear.  You're deleting data?  What data?  Nothing could be more vague than "data".

    - My proposal: that call deletes the channel's spaces but keeps its local space, which then records that the copy was deleted.

If you're deleting the channel then you need to clean up everything related to it.  But I don't understand where what you're doing fits into the life cycle... it sounds like a bit of a leaky abstraction.

---

    The system database has one writer connection. A [@carbon] op in a channel's local space writes rows there (create_space, synced_by). If a founding is in progress at that moment, those rows join the founding's transaction and would roll back if it fails. This pattern already existed, but it's now easier to hit because local ops in different spaces can run at the same time.

I think what you're identifying is a mismatch between the granularity of system tables and having multiple oplogs where the end result is stored.  Would this be resolved if all @local services ran in the "system" channel?  e.g. you could still call them from within any channel's handle... and they would run on the "message bus" of that channel.  But would resolve to a single handler and single database on the local device.

I don't know if that makes more or less sense than our present model.

Another approach might be to make ALL p2panda primitives, and all @carbon calls namespaced by the channel, whether that's a replicated channel or a local one such as system.  Then I think you could have more of a 1:1 match between oplog and database.

---

    the system database's record of which spaces this device holds is written outside any system op, and in three places.

Yep, that's bad.

    All of them write over the system database's one writer connection. So whatever system op is applying at that moment, such as a founding, silently takes those writes into its transaction.

It sounds like transactions and the actual writes aren't being coordinated in any meaningful sense.

    It's the same kind of problem as carbon_published, and the rule underneath both is: a database that an op log applies into should be written only by that log's ops.

Hmm... I lean towards thinking that [oplog, results, errors] should all be internal state managed by ServiceSpace.  And then if local services are just the same abstraction running in a different mode... I think you get clean encapsulation of the necessary state, and ops always match database transactional context.

Then you would just have [@carbon] attached to each channel and it's not special in terms of the way its state is managed, or how communication is mediated through the channel.

---

    Code outside the space machinery writes a store directly

Can we eliminate that by consolidating the ServiceSpace abstraction as I have suggested?

    The record of which spaces this device holds lives in the system database, not in the channel that holds them.

Can we eliminate that by making "system" just a local [channel] instance?

    There's already a pattern for coordinating these writes: the OpWriter

I hear you but I think it ends up encapsulated within ServiceSpace, which is the more important thing to talk about right now.

    Under your model "published records and evidence" go back into the space's file, written by the space itself, and the sync file goes away

Sure.

    Founding becomes a sequence of separate ops. It would be a system op that calls [@carbon] on each new channel's handle. Each call is its own op, so a founding that fails partway leaves channels to clean up afterwards; the identity is still listed only when it's whole.

Ok.  But you could still wrap that up in the API for a single @carbon call, and watch/wait for success, and clean up partial data if the call fails, right?  It's not as clean and robust as a transaction... we've given up transactional semantics between services.  But I think we are more fully embracing the actor model, and I think we're going to find that there's power and flexibility in that.

---

    1. Consolidating ServiceSpace removes the direct writes

Great.

    2. Making the system a local [channel] puts that record where it belongs

Agreed.

    Only two facts about a space are this device's alone: whether it holds the space,

Yes.

    and which identity's node syncs it.

Not sure what that means.

    So the record is either [@carbon] tables in the channel's local space, written by [@carbon] ops there (creating, loading, removing), or derived from [channel]'s services and refs plus the files present.

If you're following what I've proposed, there is no sense of any service writing to a different service's oplog directly.  Even @carbon.  So "the channel's local space" only has [channel] related Ops.  @carbon ops would go in:

[channel]
  |_ [@carbon]

And that would be true regardless of whether we're talking about "system", or "identity", or any other replicated or local channel.

    Founding as one [@carbon] call

The underlying need is that sometimes ops in one service trigger side effects to occur in other services.  And we need something like the "causal bindings" which I formerly described in perspectives/brandon/channels-scratch.md when we were talking about its potential for being used to revoke JWT at the channel level, cascading downwards to services and sub-channels.  Maybe the causal binding is more generally useful, and we could use the channel oplog to coordinate amongst its children and provide the rollback semantics we are seeking.

---

    a channel is reached from one identity's own channel, so whose tree it's in already says which node

Yes.  I expect that all records will be namespaced under identity, whether they are files, database records, etc.  Once you know the identity, you should also know the device, because there's only 1 device record per physical device, per identity.  Therefore you know the p2panda node.

    The design says "a rewind never leaves its service" and "atomicity ends at the service"

If you tie your legs together with bailing twine it will be difficult to make it over the fence.

    - A. Causal links. A child op records the parent op that caused it. If the parent is refused or undone, the child is undone on every replica. That's true rollback across services, but it changes the rule above, and a rewind then spreads across services.

Yes.  Now I would say that in essence the relationship is between [@carbon] and [identity], let's say.  But if you were to allow causal relationships from any service-to-service, then all services would have to watch each other.  It becomes untenable.  So if you instead say: only the channel can initiate a causal link to its children, then a service only has to watch its parent.  @carbon calls out to [channel] as a side effect and initiates each subservice call with the same causal or transactional ID.  Then a rollback can be heard by the child services, as you say.  The [channel] offers something that individual services could not: the ability to coordinate between multiple services.

    Children watch the parent's committed state and follow it.

I think that's a valid implementation... I'm not necessarilly set on whether this should be "services watch the parent oplog" vs "services watch the parent's committed state".  Although I'm not sure that writing these ops to some kind of state really makes that much sense... They don't have an "application" in terms of [channel], [channel] is really just storing them to be delegated.  I'm thinking more that services would "shadow" the parent channel, and would copy ops that are addressed to them into their local op log.  Does that make sense?  Is it a sufficient abstraction to accomplish this, and other needs for cross-service synchronization?  Is it simple enough, and could we create consistent well defined behavior for it?

---

    Shadowing makes sense and can be well defined, because the copies can be derived instead of sent.

Should we create a special kind of op that is a "shadow"?

    The rules that would make it consistent:
    1. Links only flow down

Yes.  Already agreed.

    2. A copy keeps its parent's envelope and causal id. It sorts into the child's shared order like any op, and late arrivals are handled by the rewind that already exists.

Agreed.

    3. A copy's fate follows its parent. If the parent op is refused, or rewound and refused at its place, the copy is undone in the child by that same rewind.

Agreed.

Let's talk further about: The system database is a local-only channel

---

    Down-only links don't give all-or-nothing.

I think the child services need to report their results back to the channel, to be recorded in the channel's results.  Can the channel then trigger a rewind in the child ops that succeeded

    - Sub-channels can't always shadow their parent. A team's members may not hold the company channel

Uh...

1) You can presume that some company infrastructure can see both the company and the teams, and it will be able to sync them
2) this probably necessitates a complete copy, not merely a reference onto the child node.  But still should contain a link to the original, so it will not be duplicated.
3) This is thinking down the road to more complex scenarios.  Doesn't need to be fully thought out right now.

    - How [channel] addresses its children: calls listed in the op's input, or calls its handler emits as it applies.

Uh... more like this:

channel:
    POST [books]/years/:year/close {year: 2025, tx: 'abc'}
    POST [chat]/message {body: '2025 books are closed', tx: 'abc'}

IE: the ops are exactly what you would send anyway, they just have a `tx` field that binds them together.

I guess the alternative would be to put them all in one Op, similar as we do with sub-calls?  That could work, but might make listening/routing harder.  And you would be waiting for them all to resolve locally before publishing the combined op in [channel].

I'm reading your more recent responses now.  You used the word "cause" which I like... my `tx` should be renamed to something in line with that vernacular.

    The system database is a local-only channel

Whoa I definitely don't want to talk about that yet.  Stay with the present topic.  What's the topic number for what we're talking about?

---

    the [channel] op can't hold its transaction open until the children answer, because the children derive their copies from it once it's logged. So it commits first.

Still feels a bit sticky.  Like our lifecycle doesn't quite match the natural lifecycle of an op, which are record first, append only, and record results in a separate table.

Also a significant amount of the traffic may end up in the [channel] rather than the services.  Dunno how I feel about that.

---

    1. Recorded first. The OpWriter appends the row before the op applies (crates/core/src/ops/op_writer.rs:23).

I think you're back to "record in channel as multiple ops", because it has to be recorded the moment the source service calls it, and we don't know how many sub service calls the caller will make, or in what order, or whether it has conditional logic that changes what is called.

    2. Children copy it from the log and apply their copies.

Yep.

    3. The cause op applies by awaiting their outcomes. It holds its turn while it waits, the same way an op holds its turn today while it awaits a call effect.

Well...

1) the sub-services know not to publish yet, because it has a "cause"
2) [channel] can't know when it's complete
3) only the caller can determine whether it considers the result a "success"
4) the result should be logged in the results/errors table of the [channel]

    4. Its result goes in the results table: applied only if every child applied, otherwise refused.

I only disagree with "every child applied" - you need to rely on the business logic of the caller for this.

    5. "Publish only once judged a success" needs no new rule. Only applied ops are published, as with every op.

Agreed.

    Two costs, though:
    - Turn contention.

I don't think you run into this in the same way if you split sub-ops and commit each one.

    - Storage. Each device stores a cause's calls twice: as the cause row and as the copies.

Mmm... we need to get clear on it.

A) in normal scenario, channel and child services are 1:1 in membership, you can just store a pointer to the [channel] op.
B) in your scenario you proposed where someone is on the team but not in the company, and an op comes down from the company... solving the problem by copying assumes that the [service] is the vehicle for replication, not the [channel]... so I think this is internally inconsistent.
C) it's not actually a problem of "have the service but not the channel".  It's "have the channel but not the parent channel".

IE: its propagation from parent channels that you really need to solve, and THAT could be done by copying (with some proper AUTH proof that doesn't require visibility to where to original op came from).  And you could leave the channel -> service link as a pointer.

This feels more clear, and less weird.  What do you think?  This could mark the pattern for how sub-service calls should work: they should be mediated by the channel, having a single "cause".

---

    1. Completeness needs a closing record.

Yeah, but that's accomplished through results, not through the oplog.  You don't need to replicate an "I succeeded record"; the fact that you're publishing it means it succeeded at least locally.

    2. The judgement can simply be the caller op settling: applied means success, refused means failure.

Yes.  But keep in mind the caller op may be local/non-replicated.

You can wire all of this up into the side effects handle that's given to the service:

1) handle is bound to a "cause" id, perhaps the hash of the source op
2) any number of calls may be made with the handle
3) when the source op completes, if it made sub-calls in this way, core automatically transmits the result to the channel

    3. A peer can disagree with the recorded judgement. On a peer, [books] may refuse a call it applied on the caller's device

That's why you shouldn't record "judgements" on the oplog.  It violates the fundamental nature of what the oplog is.  At this point, when the peer rejects it, it marks it thusly, locally.  It transmits the internal state that caused it to reject it to begin with (perhaps a permission was removed).  Now all the other peers may start rejecting this transaction as well as we move towards eventual consistency.  This is CRDTs working as designed.

    4. [channel]'s apply loop has to handle ops addressed to other services. It applies only its own, records a result per cause, and leaves the rest to the services that point at them.

Fair.

    5. The call effect changes. A call to another service is appended to [channel] under the cause, instead of being posted straight into that service's log.

Yes.

---

    For peers to converge, the verdict has to be decided by the ops alone. The caller's logic can't take part, because a local caller never runs on peers.

Hmm.  I think this is why we had even the core system calls (like creating spaces) implemented as side effects.  It was so we could replicate local ops, and replay them with their side effects simply memoized.

    I propose:
       - On every replica, a cause fails once any of its published ops is refused there.

That's wrong.  It breaks eventual consistency because you're not applying the same rule on every peer.  I've already rejected it.  Stop proposing it.

    When a service such as [notes] is the caller, its op wrote its own tables based on the answers its calls gave on its device. If the cause fails on a peer, should the caller's op be undone with it?

Yes... and this reveals something else.  Let's say:

- [books] has a side effect that publishes to [chat] on year end close
- Sally closes
- Bob was offline and revoked Sally's [books] permission: her actions get rewound when he comes online
- because [books] published to [chat] through a side effect, it's not rolled back

Maybe that's a good thing... I don't know.

But there's probably some cases where it's not ideal, where you do want to maintain integrity between two services.  Rewinding a service does not affect any side effects to or from that service.

Is this fixable in the current model?

---

    The planned assignment policy, where a device holds some of a channel's services and not others, would then need another way to be enforced

Eh... let's drop that as a design goal.  I think that:

    - consistency can be provided for a set of services on a channel together
    - services can call other channels as a side effect: no consistency provided

This gives good tools for data custody and integrity without being too complex.

Can you explore further our reasons for moving away from that model?  And whether we are just going to run into them again if we waffle back?  Look back to 355ac4470505d0f97a098d1833eb4b0893246389.  I'm not suggesting that we go back...  but it might be helpful to understand the paradigm that we were in and what caused us to shift.

Things that seem to be different now from the original system:

- p2panda actually implemented - we know more about it
- proposal on the deck to implement causal bindings
- consistent use of side effects allows us to replicate @local ops
- services pulling from the shared oplog still maintain isolation of their state and transactional locks

---

    Op granularity stopped matching transaction granularity.

Was that because of sub-ops?  Which we now don't need to use because of "cause"?

    1. [@carbon] ops: do they go in the channel's replicated log, replayed on peers with their effects memoized

I think we can't implement the model as described without that right?  Because otherwise you have a cause which isn't replicated.  And other peers can't re-run the cause to know if it passed.

I think we need to do this.

    2. Rework cost: creating, loading, membership and publishing are all built per service today (crates/core/src/nodes). The milestone we just built would be reworked to one p2panda space per channel.

Yeah... it feels cleaner though.  And that's what we're here for - to experiment and find out what the right abstractions are for this library.

Also, should we keep the name channel, or name it a space again, so that we don't need to reinvent p2panda terminology?

---


