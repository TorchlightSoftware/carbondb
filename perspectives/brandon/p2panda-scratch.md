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
