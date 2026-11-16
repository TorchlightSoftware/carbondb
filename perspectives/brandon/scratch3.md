/agenda new refactor codebase before adding replication

1) Don't put my name "brandon" in the tests or the codebase.
2) do "builtins" need to be stored in a form that's more hashable?  Same question for SQL migrations, both for [@carbon] and for [space], [identity], [device], etc.  The call catalog, the handler implementations, and the SQL schemas all have to be tightly versioned, and you can't run a CALL on an unknown version of code.  So it seems like we need to implement and enforce this abstraction and packaging, even in our own impls.  And it can't be language specific.
3) space.rs: it's pretty ugly to have SQL interspersed with Rust code
4) carbon.rs: weird to see call_system, call_space, etc. define their own keys/handlers... I would expect to see more of a structured "routing in one place", "handlers in one place" etc rather than scattered throughout the code.
5) create.rs: this file is really weird and seems to completely violate separation of concerns.  I suggest demolishing it and letting the functions live in identity and space files as appropriate.
6) Why is there no identity.rs, given that is an important domain and architecture term?  That's a little bit weird.
7) store.rs: is this our wrapper around SQLite stores?  Is it the thing that lets us rewind?  Is that implemented now, or TBD?

   - The per-space queue lives in the JS runtime, not the core. implementation.md gives the queue to the core. It's fine for single-node, but replicated mode will want it in Rust, where synced ops arrive.

Yes.

   - Manage lists hold only identity ids or self. Entries naming a company aren't expanded in single-node.

Ok?  I don't think I want to expand companies, or that's not something we had talked about.  Doing this with identities is quite sufficient for now.

   - createIdentity returns no paper key yet, because recovery isn't designed.

Add to agenda.

   - The "someone else's message" path (403) is untested. In single-node only one identity can reach a space, so it needs replicated mode to test.

Add to an appropriate story where this will be implemented?  Or to agenda if there's nothing existing?

   - The native build only copies a Linux .so. Per-platform packaging isn't done.

Add to backlog.

   - Peer-only calls reject with 501 SINGLE_NODE; manage changes and data deletion with 501 NOT_BUILT. I chose 501 so these read as "not available here", not as bad input.

Ok.  I don't have a problem with this per se... but generally I think the organizational strategy of our routing needs work as I mentioned above.  Things that look like routing seem to be scattered throughout the rust code.

---

    Topic: Builtins and services need one hashable, language-neutral, versioned package form
    Key question: what does "can't be language-specific" mean?
    - Reading A: the mechanism is the same for every package

I mean this.

    The stories README already promises this: "every implementation of a service … must behave identically, whatever its language"

Can you get rid of that?  It's obviously confusing to you, and it doesn't do anything for me.  Trying to create language independent implementations with byte identitical outcomes is a goal of exactly no one anywhere.

---

    stories/implementation.md:310 says "handlers are language agnostic". I read that as "CarbonDB doesn't care which language hosts a handler", which fits reading A

Yes.

    Next question: what does the package hash cover?

Well specifically (and I have already stated this): it means the list of calls a service offers, any schema definitions, and handler implementations.

If it were a directory: there's no canonical way to hash it, or to run it.

If it were a zip file: you could hash it, but still no canonical way to run it.

Could we say that services are always WASM?  Or WASI?  Can existing forms like Otto be compiled down into that form factor?

What I'm wanting here is a form that can be serialized, transferred over the web, hashed deterministically, and has a known entry point.  But it does have the disadvantage of being more particular about the form that services take.

In our design up to now, the service runtime is completely open... we could publish bindings for different languages and in short order they become operational.  The serializeable form would put the power in the hands of CarbonDB to run these services...  It would mean we could support our own app store of deployable software, and have a built in transfer mechanism for it.  It would mean that installing services on one node, they become syncable to all peers that share a space together.  And I think that's probably necessary for the full vision to be realized.

So:

1) how much trouble would it be to convert existing forms to this?
2) is it worth the trouble to get dynamic deployment of packages?

---

    - Javy, which runs JS on the QuickJS engine. Each service module is 1–16 KB, plus one shared engine module of about 870 KB.

Sounds nice.

    - Database: drizzle already has a proxy driver that sends (sql, params) to any executor. That executor would become the host's db import, the hook CarbonDB gives a module to reach the space's database.
    - WebCrypto: jose and a few files (core/candle-service/src/seal.ts, the webhooks, the auth hashing) use WebCrypto, which QuickJS doesn't provide. Those would need host-provided crypto.
    - Integrations: Twilio, Square webhooks and fetch are non-deterministic I/O. They have to move out of handlers into the app regardless of WASM.

I think we will have pains converting this codebase.  I think we should provide dynamic loading of WASM, and it should be the preferred format.  We should also let the host init whatever services they want: backward compat with any services that would be difficult to convert to WASM.  But: we still have a question of how do we establish a canonical hash of (handler, migrations).  Elsewhere we proposed just tracking # of migrations.  So maybe all we need to hash is the code and give a migration count.

    On iPhone, JS would run inside an interpreter inside an interpreter.

That doesn't sound great.

    - It reverses agreed story 04.

I don't care.

    - Who may install a package into a space?  Probably manage.

Yes.  Clearly.

    - Does the local [@carbon] service stay native?

No, I would want to convert all of our provided services to the WASM format, because it becomes much easier to track handler version and migrations.  This is what originally brought this up for me:  I was looking at our new services and realizing if you had two nodes communicating on the network we have no way of seeing whether they are running compatible versions.  I suppose you could do that with the versioning of CarbonDB itself... But it seemed like, we're going to need to implement that for all services, so why not dog food it?

---

    1. The tarball must be self-contained. A tarball doesn't include its dependencies...

I don't want to nitpick about this.  I'm not really going for an "automatic download and deploy" with Node.js packages.  I just want to support them with manual/out of band install.  But requiring the packaging gives us a clearer target to read from.

    2. So "manage installs it and it syncs to every peer" would mean running unsandboxed code from peers.

Yeah, we're not doing that.  Only WASM services are supported for automatic download.

    3. A Dart or Swift app couldn't join a space that uses an npm service. Spaces meant for every platform need WASM.

That's really obvious.  Do you need to say that?

What I'm looking for with the NPM version is:

1) the app/instantiator of CarbonDB is responsible for installing packages
2) they're also responsible for registering them with CarbonDB
3) CarbonDB doesn't take responsibility for downloading or even finding the packages - they get imported as normal
4) CarbonDB simply accepts field parameters during registration for the package hash and number of migrations

With WASM, the only thing that's different is instead of providing an instantiated node module, you provide a path to a WASM file and CarbonDB does the loading.

So maybe we provide:

    import books from '@tlsft/otto-books'
    carbon.services.loadNPM(books)
    carbon.services.loadWASM('./path/to.wasm')

Both packages, NPM and WASM, need to provide access to migrations at a canonical location.

---

    carbon.services.add_npm('books', books, hash)
    carbon.services.add_wasm('chat', './chat.wasm')

Probably needs to be that.  And if the migrations are at a canonical location, then we can take the count directly from there so we don't need that as a separate argument.

    New open questions from this:
    1. Where do an npm package's migrations live? My proposal: an exported migrations list in the npm module, and an export (or a custom section) in the WASM module.

Yes.

    2. Where does an npm package's hash come from? Either the package exports a field that its build writes, or the caller passes it to loadNPM.

1) is it possible to export a function that calculates this automatically?  Obviously you can't include the hash plaintext, because including it would... throw off the hash.
2) is there existing tooling for grabbing this info?  Can we grab it without interacting with NPM?
3) would it be best as a post-install hook?

    3. Where does a service's name come from? loadNPM and loadWASM no longer take a name the way services.add('chat', impl) did. Should the package carry its own name?

I added the name back.  I didn't mean to lose that.  On the other hand, yes, packages should carry their own name at a canonical location ".name".  At some point we may change the interface to rely more on that, especially with auto-loading of WASM.

    WASM package's version is the hash of its bytes, computed by CarbonDB as it loads the file

Yes.  What are the other open questions?

---

    npm records each installed package's integrity, the SHA-512 of its tarball, in package-lock.json and in node_modules/.package-lock.json. Both are readable without contacting npm

Yes, that's what we should be sourcing from.  We could offer a utility to help look this up and inject it into the field?

    Workspace-linked packages have no integrity

We can change Otto's practices around that, you don't have to contort CarbonDB into a pretzel to try and support it.

    It covers only that package's tarball. Its dependencies have entries of their own.

Just use what NPM gives us.  I don't want to complicate this, and I think you're reaching at an extreme edge case for where we are right now in the development process.

    1. What does an op carry? One package hash per service, or one per call? Alongside the migration count, now that the count comes from the migrations themselves?

It's per call.  Per service doesn't make any sense Claude.  There are no services in the oplog, that's gibberish.  So the Op must carry package hash and migration count.

    2. What does enforcement look like in single-node? No ops arrive from elsewhere. Should a device refuse to open a space whose recorded package hash it lacks?

Uh... I don't want to create provisions elsewhere in the code for hash to be empty.  So no, it's required.  They can put gibberish or a static string in the field if they desire.

    3. How does a space upgrade to a new package version? Does a package name the versions it supersedes?

Backlog, don't ask me now.

    4. Which privileged host imports does [@carbon] need? It creates spaces, generates keys and writes the system database. How are those kept from installed packages?

What are you even talking about?  Speak more clearly.

    5. Can iOS avoid the interpreter for packages shipped with the app? That would mean compiling them ahead of time into the app, leaving only packages installed at runtime interpreted.

Backlog.

---

    [@carbon] needs more powerful functions:
    - create a new space's database file;
    - generate a device key pair;
    - write the table of identities on this device, which holds the secret keys.

No, why would these go on @carbon?  You're right that you need to inject them into the WASM context, but conflating that with "things that should go on the @carbon service" is I think incorrect.  In addition, a lot of this stuff shouldn't be provided directly to the service, it should be set up for it as part of the init process for the space.  What the service is looking for is bindings to existing, manifested storage.  See wrangler.toml files in Otto for what those bindings look like.

---

    A Worker gets a ready-made database, not the power to create one

A Service in our case.

    Open question: which handles exist beyond a space's own database, and who grants them?

I don't think you're asking the right questions, or thinking about this the right way.

1) the service needs to declare its config and requested bindings.  The wrangler.toml format is acceptible for this purpose: let's just use it and thereby create some compatibility.
2) the package (NPM or WASM) will have to expose this.  I think it should be baked into the package because a service that asks for different bindings is fundamentally incompatible/not the same version.
3) the carbon runtime is responsible for looking at the bindings and accomodating them, and namespacing appropriately so that there are no naming conflicts.
4) Any bindings code is an internal carbon API, called as a result of creating a new space, or adding or updating services within a space.  They are never called directly.

---

    1. Merging environments: Otto declares its bindings only under [env.development], [env.test] and [env.production]

The process that generates the package should be run in a specific mode (production in this case) and should only generate that portion of config.  The resulting config should not be namespaced by environment.

In addition I agree with eliminating fields that we do not process: but there should be some kind of warning for this so they're not silently dropped.

It looks like the main things we have are ENV variables, d1 (SQLite), and r2 (Object storage).  We should be able to support all of those in a CarbonDB context.  We don't have to tackle r2 right now, we can backlog that.  d1 and r2 should be translated to non-cloudflare names.  `sql` and `file`?

    2. Choosing the migrations folder: dev and production point at different folders

Production.  We will build a CarbonDB specific package for NPM, and everything will be production version.

And for the Rust/WASM services to be included with CarbonDB, they can just include one toml file: more straightforward in a way.  CarbonDB doesn't require per environment configs because the whole space and every service is state and namespace isolated.

---

    each sql binding gets its own database file per space

Yes, and I think we've already covered that.

    I propose the op carries one count per sql binding

That's fine.

    When does a device set up a space's bindings? I propose it does so when it applies the op that creates the space or changes its services.

Yes, I think we've also already covered this.

---

    Open question: do the queries leave Rust as well

In terms of general service packages: CarbonDB is unopinionated and doesn't control the internal organization or contents of services.

In terms of the packages that we will be implementing: They're short, you can just leave them inline in the handlers.

---

    - Splitting create.rs: the setup work (key pairs, space files, identity records) goes to the core's identity and space modules. The call handling goes to [@carbon]'s package handlers.

Ok.  How will the service calls inside [@carbon] access the carbon internal APIs?  Do you offer a binding of some kind for this?

---

    Through host functions: a binding is a set of functions CarbonDB links into the module

You verbosely duplicated a bunch of information I already know, and then didn't really answer my question.  So I'll ask again, differently: Many of the calls in the @carbon service directly access p2panda instance, or other in memory objects which are not SQLite tables, and are not ENV variables.  I'm asking how you will access these, and I don't think you've answered that question

---

    My proposal: [@carbon] stays host code in Rust. It's the device's control surface over p2panda, keys and setup, not a package. [space], [identity] and [device] still become WASM packages

Ok, thank you.  Now we're actually on the same page, and I'm with you on this.  I want to visit the versioning on [space], [identity], and [device] one final time though: do these need independent versioning, or can they be implied by the CarbonDB version?  I assume (and prefer) that they would be packaged inside the CarbonDB binary lib.

---

    Putting [space], [identity] and [device] inside the CarbonDB library and versioning each one separately don't conflict. The .wasm bytes can be compiled into the library, for example with include_bytes!, and each package still gets the hash of its own bytes.

Great.

Everything else: agreed.

---

I'm curious about Lane as a concept within Space.

1) what informed the naming?
2) it seems like it's not actually exported by the node wrapper but is an internal implementation detail?
3) why is it in the node wrapper and not in internals?

    - Two parts of routing stay in JS, deliberately.
      - isBuiltin decides whether the core answers a call or an app service does, by service name.

What I was asking you to do was unify routing.  It doesn't sound like you did that.  Core for some reason gets special treatment and isn't dispatched in the same way.  Can you explain why?

      - The "registered services" check for POST [@carbon]/space stays in JS too, because only the runtime knows what's registered.

It's bizarre to try and untangle what you're saying here:

1) yes, services should be responsible for their own internal dispatch
2) I think you're saying [@carbon]/space routing is written in JS
3) But isn't the implementation written in Rust?
4) Are the handlers what you're talking about when you say "runtime"?  I would think "runtime" would mean the CarbonDB runtime in this context - the internal implementation, not any handlers.
5) I go back to "I asked you to unify routing.  It doesn't sound like you did that."

    - One rule the code relies on: a job must never throw into Rust, or its side of the space stops. The runtime catches everything a job throws, and a comment in the binding states the rule.

What's a job?  This is new terminology you've introduced without any discussion or agreement.  Along with Lane.

---

    a space has a queue that holds local calls and synced ops, which are ops

These are terms I recognize:

    space
    oplog: is this what you're calling a "lane"?
    op
    service call: is this what you're calling a "job"?

Is that right?  The new terms feel very muddy and distracting from the original domain concepts.

    Why it's in the Node binding. What waits in the queue is a JS function, and an item is finished when a JS Promise settles.

Ok, so this is part of the binding layer where we're waiting for promises?  You can just dispatch to the function and put the resulting promise in the queue, right?  I'm also not sure why the queue exists at all, but maybe it's a common facet in node->rs boundaries like this.  Is there one queue for everything?  Are queues instantiated per service call?  How do you address them or keep them separate so the results remain associated with the correct call?

    But the result goes against the design: implementation.md:138 says "The core owns the order."

Yeah... we're implementing CRDT semantics here Claude.  It sounds like what you've done is pretty far off target.

   - Once p2panda delivers synced ops in Rust, they have to join that same queue without going through JS. With the logic in the binding, they can't.

Yes.  Which goes back to what I've been repeating, over and over: build a unified dispatch for ops.  Are you starting to get the message?

    Proposal: the core owns each space's queue, the services this device runs, and routing; the binding only translates

    - Routing in one place. The core keeps the list of services this device runs: the builtins, plus app services registered with a name, migrations and an entry point. Carbon::call routes every call by service name. A builtin goes to its Rust handler. An app service goes to its entry point: for an npm service, the JS side's apply(op, db) (implementation.md:138), and later a WASM package. Each entry point routes its own calls, so the JS calls map becomes the inside of one npm service's entry point.

Yes.  This is what I keep asking for.  I don't know how it keeps getting lost in translation.  This is what's holding us up from proceeding: it needs to be unified before we implement p2p, or we will get a multiplicitive explosion in code paths.

    - What JS loses. isBuiltin, the key-string pre-check, and routing in #serviceJob. [@carbon] checks registration itself, and app-service migrations run in space::open alongside the builtins'.

Ok.

    - One queue per space. It holds ops, both local and synced. The core tracks whether an op is being applied and starts the next one.

That's an oplog Claude, isn't it?

    The binding hands the core a way to start a handler and reports back when its Promise settles; that part is the hard bit to design.

Consider functional programming, callback style.  Talk through it with me if you have questions.

    The names Lane and Job go away.

Yes.

    - Reads take their own snapshot and don't queue.

Yes.  That's what our design spec promises.

    This needs more than one reader connection, either one per GET or a small pool. Tracking which tables a GET read in, so one connection per GET may even make that simpler.

Ok.  I'm fine with either way.  At this point we're trying to establish a "correct" implementation more than an "efficient" one.

    - Agenda. Reopen "Routing in one place, handlers in one place" and "Move the per-space queue into the Rust core". Both are marked implemented, but by this reading they aren't.

Eh.  Sure, but clean up the cruft if you're going to reuse that topic.  Make sure it reflects current decisions.

---

    The queue is the oplog's unapplied tail

Then call it a RunQueue.

    A local call's reply resolves the caller's Promise; a synced op's reply only records the outcome.

Regardless of whether its local or synced, we run the handler locally, right?  So I don't see why a local call is any different from a synced one in terms of the mechanism that runs and waits for it.  That's the whole purpose of the architecture.  CQRS is what Martin Fowler used to call it way back.  Reactive is another name in the same vein.  Don't split hairs with me on naming or history, I'm just trying to say that this is a well established architecture, and local shouldn't be different from remote in the way it's applied.

    settle(result) commits or rolls back, records the outcome in the oplog

Are you saying the oplog is responsible both for storing incomplete calls, and for storing the results (output) of those calls?  That feels like it violates something fundamental about the model: The append-only nature of the log.  Conceptually what you're doing is:

    sync  -- OPLOG (calls only) --> local node
    local -- OPLOG (calls only) --> local node
                                    local node -- run handler --> SQLite
                                    local node <-- results --     SQLite
    local <-- RESULTS --            local node

Technically, you could put RESULTS back into the OPLOG as the result of the CALL, but you would be treating RESULTS as a separate, local only value.  It's not part of what you shoot out in the sync transmission, right?  Unless you're sending it out so that when other peers run the handler locally they have something to validate against...

We do run handlers locally BEFORE syncing the op, and we abandon the network call if the handler failed locally.  So I suppose we could bundle the result and provide it for verification purposes to peers.  Just want to be clear on what we're doing here.

    Rust and JS take the same path

Definitely manditory.  And you could add WASM into that statement.

    A Rust builtin calls settle

I'm less opinionated about this exact implementation.

    The binding makes settle a JS function from a Rust closure.

Ok.  It sounds like an abstraction that is much easier for you to translate through the JS binding.

    GETs use the same entry

Ok. Just to make sure: when we talk about GETs using a snapshot, they need to run within the transactional context of the call structure.  They can't just load a snapshot of the state before the call started.  Otherwise calls that build on and query the state that was just modified won't work.  And this needs to be applied recursively: another GET service that's called within a surrounding service needs to be wrapped in the appropriate transactional layer.  We should have test cases for these things, and for failures and rollbacks.  It's very important that transactional semantics are implemented correctly.  That must be well tested before we go into P2P or things will get crazy fast.

    [@carbon] writes replicated spaces directly, with no op, which is a second dispatch path

You are using ambiguous language for this.  Are you talking about ops that talk to [@carbon], or are you talking about a [@carbon] call issuing a sub-call to [space] or [identity]?

    - POST [@carbon]/space inserts the new ref straight into the parent space (crates/core/src/builtins/carbon.rs:93).

On the call to @carbon: I'm not sure how you implemented this.  Either: A) @carbon, or any other @local services, need to not go into the space oplog and get replicated, or B) they need to be marked with the device that ran them, and other devices seeing this do not run them.  B is probably somewhat better from a record keeping perspective: it shows us all the actions that resulted in current state.

On the sub-calls to replicated services: Those need to be replicated as normal.  We talked this to death, and I was very clear in my design.  There should be no special "send it directly to the parent space without landing in the oplog".  I don't even know what that would mean semantically.

    - Creating an identity writes the identity space's origin and devices the same way (crates/core/src/identity.rs:63-67).

I have no idea what you're talking about.  The code you're linking to is a struct instantiation.  But if you're somehow sending refs directly into the space without going through the oplog, yes, that's totally bizarre and wrong.

    Under one dispatch path, [@carbon] would do its local setup (keys, the space file) and then submit ops like POST [space]/refs into the space's oplog. Otherwise peers never see those writes.

YES.  We agreed upon all this.  I don't know why you implemented it differently.

---

    The oplog stays append-only and never holds results
    My proposal: when an op is applied, append it once together with its outcome, and never update that row. A failed local op is never appended.

You're contradicting yourself in these two statements.  But drop "never holds results", and I agree with your proposal: put the output of the Op in the op record.

    I'd recommend not sending results to peers for verification
    Before replicas converge, they can legitimately apply ops in a different order and get different results, so checking against the author's result would raise false alarms.

Fine.  I don't want to argue for now.

    - Plan: [@carbon]'s calls to [space], [identity] and [device] become ops in the target space's oplog, as stories/services.md:22 and :25 already describe.

You're going to do what I asked in the first place.  Yes, please.

    Nested GETs read inside the transaction around them

Yes.

    Open questions:
    1. Where are ops on [@carbon] recorded? The docs say the system log, one row per op naming its space (implementation.md:96, services.md:18). Your option B puts them in the space's oplog, marked with the device that ran them, and other devices skip them. Do we switch to B?

Do you agree that it's better to have record keeping replicated?  Or do you think it should stay on the host for some reason?  If you don't have anything to add and you agree with me, then record it, and quit bringing it up.

    2. When a replicated handler calls another replicated service, say [chat] calling [space], does the call run inline? My proposal is yes: inside the caller's own op, under a savepoint, and every replica re-runs it as part of the outer handler. Only calls made by a local service would become new ops, because a replicated handler that emitted ops would duplicate them on every replica.

You're talking about making Ops atomic.  Ok.  I hear you, but it means we have to offer a separate non-replicated call function, or a boolean flag for non-replicated calls.  And it might be nice to record the sub-call tree, local only, just as we do for the results.

Other than that I feel it's fair to do so: the top level op is the only thing you need to replicate in order to come to correct state.

    3. Is an op appended once, when it's applied, with its outcome, and a failed local op never appended?

We already covered this.

    4. Do authors send their handler result with the op for peers to verify? I'd recommend not, for the reason above.

Already covered.

Is this clear?  There's something else I'd like to talk about.

---

    Ops on [@carbon] go in the space's replicated oplog, marked with the device that ran them. Calls on the system handle, such as POST [@carbon]/identity, have no space, so they stay in the system log.

Ok.

    Two cases come up, and the core can tell them apart from who is calling:
    - A replicated handler calls a replicated service: the call runs inline, inside the same op.
    - A local service calls a replicated service: the call becomes a new op.

If you create one API function: "call", and you pass that to UI, sub-service API client, and local services like @carbon, then how do you "tell them apart"?  You say you can, but you haven't described the mechanism.

---

    when the core starts a handler, it builds a call tied to that op and its open transaction, and hands it over along with db and op

Ok, I'm satisfied by that.

Next subject: service encapsulation.

Rename the directory:

    sql -> migrations

But also: very questionable for migrations to be in a folder all by themselves, instead of creating folders for each of the services.  Despite having numerous examples for what a service is (look at Otto for instance), you still didn't encapsulate their internals.

You have a completely different compile path from core, right?  With [space], [identity], [device] compiling to WASM?  That's not clear at all from the directory structure, and it should be.

What about putting a `services` folder under `crates` and go from there.

---

    1. Do the service crates build to WASM now, or link natively into core until the WASM host exists? My proposal is to link natively now.

Do that.

    2. Does [@carbon] get its own folder inside core?

Yes.

Ok, are you clear on what needs to be done to accomplish the refactor into single dispatch, and encapsulated services?  Ideally I would like all that to be implemented, but I'm fine if you separate it into:

1: single dispatch
2: service encapsulation
3: wasm

With review steps in between each.

---

    A failed inline sub-call throws to the handler that made it

Yes.  You'd be changing the semantics of existing services otherwise.

    My proposal is one file per space, with tables namespaced per service

Certainly fine now, and I think it may even be the right decision long term.

Ok, proceed with phase 1.

---

    - New interface crate, crates/service-api: this is everything a service is written against.

Hmm... I'm not sure why you made that a crate?  I don't think "service" is a Rust abstraction?  Please explain.

    - crates/services/{space,identity,device}: each depends only on the interface crate,

Interesting... what would an NPM package use?  I suppose we would have the equivalent of a Node.js interface?

    - Frame: the core's per-call struct is now Frame (crates/core/src/frame.rs), which implements Invocation

You're introducing more domain logic we didn't agree upon.

    - [@carbon] calls one function in the [identity] crate directly: carbondb_identity::id_of. It needs the new identity's id before it can author that identity's ops.

Does identity have local state outside of that which is replicated through its `space`?  I'm thinking it probably shouldn't, but open to discuss.  Where is the data for `carbondb_identity::id_of` stored?  Moving something identity specific into the interface also feels a bit weird...  Please explain more.

    ask only works while every builtin answers at once. That holds for Rust services, and will for WASM ones, whose calls into the host are synchronous. A builtin could never ask an async npm service this way.

You're again referring to internal jargon that you created, outside of the language and calls we agreed upon.  I assume by this you have some `ask()` function which is used somewhere in the delegation chain.  Why didn't you build it to be async compatible?  You know that's a requirement for 95% of services...  I guess you could wait until we're supporting [chat], as long as you haven't backed yourself into a corner.

    - Two folders are both called "services". crates/services/ holds the builtins and services/chat holds [chat]

That's fine.

    - Tables are not yet namespaced per service in the one shared file. Builtins keep their carbon_ prefix by convention. A service also declares at most one sql binding for now.

I think you should implement the namespacing now.  And also why don't you just include the binding name in the namespacing path?  If we need everything to run in the same DB instance for rewind/rollback purposes, then why not just implement everything that way?

---

    - Rename it for what it is: the Rust bindings to the host functions.

"ServiceBindings"?  What you said about host functions is weird to me.  Services shouldn't be calling host functions.  They should be getting invoked by the CarbonDB runtime...

    An npm service imports nothing from CarbonDB;

Yes, that's ideal.  "service" is the lego block that receives Service Calls and performs SQLite transactions.  It doesn't know about its runtime.  That's what makes it flexible to run on different runtimes, like Cloudflare workers, Node.js, and CarbonDB.

    neither should a WASM one, beyond the host functions

I recommend that you make this a simple layer that establishes the API footprint that services must expose in Rust.  No less and no more.

    For npm, the equivalent is the object every handler receives, {path, input, db, call, op}

Yes.  Some sense of a call() function and its signature.

    Proposal: use the terms we already have.
    - The core's struct is a service call in progress: ServiceCall.
    - What a service sees is the handler context, the term stories/implementation.md:310 already uses.

Yes. Can you please store a memory to use existing domain terminology instead of inventing new terms?

    [@carbon] needs the id before writing the identity space's ops

Why are you not just returning it from the `POST [identity]/origin` call?  It feels like you're reaching for a lot of hokey solutions to avoid doing the obvious.

    ask() makes a call from inside a handler and fails if the answer doesn't come back immediately
    for a WASM handler to wait on an async npm service, wasmtime has to pause it and resume it when the answer arrives

You cannot simply violate the intentions of the design and force everything to be synchronous simply because you found it convenient to do so.

Again, it feels like you just implemented something new and weird without talking to me about what you're doing.  Why do we have a synchronous only variation of a call?  Why does it have weird semantics attached to it?  I think you're overcomplicating this, and generating confusion in the way that people will interact with the API.  Should I .call() or .ask()?  I don't understand what problem you were trying to solve.

    Proposal: make the Rust builtins callback-style too, and drop ask().

Yes.

You still have migrations under core.  I asked you to put them inside the services they belong with, and you didn't do that.  You also don't have any naming convention, any order of operations, any system for how they should be run.  That needs attention.

Let's focus on one thing at a time.

---

    The runtime invokes services, handing each call a handler context: `db` and `call`

Ah, that's what you mean.  That makes sense.  Yes, the bindings that were requested need to make their way through somehow.

I'm fine with your proposals.  I think this should be called ServiceBindings.

---

    - A. The host rewrites each statement (recommended)

I like it, and I know how we would implement it in Otto with the QueryRunner.  You're saying we can implement it in rust with sqlparse-rs.  I'm ok with that.  Seems like it should be part of the general ServiceBindings layer.

---

Regarding migrations: I agree with your proposals in general.  But I wonder: should we be implementing this in core?  Or should the service simply expose an init() function that gets called at a prescribed time in the lifecycle?  It feels like migrations are something that might be better left up to the service.  And most frameworks implement migrations in such a way that they utilize the SQL state itself to track them.  We don't really need to duplicate functionality that is already in various frameworks...  And we're probably not doing ourselves a favor by forcing implementors to fit into a shape on this.  What do you think?  Can we leave this state management up to an internal implementation detail of the service?

---

    The core counts migrations today only to version ops, and the package hash does that better

Yes.  100%

    Every table has a primary key

Why do we care about that?

    Each service stays in its own tables. The rewrite we just agreed covers init()'s statements too

Yes.

    Cloudflare D1 migrations are applied by the wrangler CLI, not by the Worker at runtime. A ported Worker has migration files but no code that runs them.

Yes.  As part of the port we need to make init() functional.

3. Does CarbonDB offer an opt-in helper that applies a directory of D1-style migrations from init()?  The helper would live in the npm runtime and in ServiceBindings, as a convenience services may use, not as part of the footprint.

Sure.

    Every replica reaches the same schema. The op carries the package hash. A replica without that version holds the space until it has it, then runs init() on open

Yeah... we can just fail for the current implementation.  But we could get smarter about "download on demand".  I think there's a bit of a snag in terms of breaking schema changes... anything that's not backwards compatible.  E.g. if you rename a column, you can't simply run the old service code on the current schema.  And you can't be reverting the schema changes, applying old ops, reapplying the schema changes, hoping the implementor provided a data migration...  That's going to amplify any migration gaps and turn them into major data loss.

So I think we only go forward with service software updates and with sql migrations... But let's say we have this scenario:

- Peers A, B, and C
- all version 0.2.0
- C is offline, with pending changes
- A is upgraded to 0.2.1, and sends an op
- B syncs the WASM service implementation, runs init(), and applies the op
- C comes back online.  Receives and applies the new op no problem.
- A and B are unable to apply C's op

A) Do we need a service call migration?
    - does this violate best practices?
    - I feel like there's probably established wisdom on why we don't do this
B) Do we follow semver rules?  Only major version is a breaking change?
    - Do we warn the user before upgrade, and then drop any outstanding transactions?
    - What if the service labels a breaking change as minor or patch?
C) Is there other wisdom/best practice on how to handle this?

---

    - stories/backlog/old-ops.md: the handler receives the op's version, and can refuse the op or migrate its input.

Ah.  I forgot that we had established that.  It's actually elegant: let the service decide how it will handle it.  But I think we should always have one version of the service running, and the service version itself decides whether it supports the op.

I also think that in addition to the hash, we should support a declared version (semver, canonically).

    A) Migrating old service calls is established practice, not a violation of it.

    C) The usual practice is "expand, then contract".

Great, add the context you're mentioning here to old-ops.md.  This should make it into our final documentation.

    - Semver stays advisory. CarbonDB doesn't trust it.

Sure.

---

    where does a refused old op go?

Gets dropped.  We need a table for such things.  There's a backlog story for an error table.

    stories/backlog/rejected-after-merge.md

But I think per the other decisions we've made, maybe this should be a per-space error table.

---

    My proposals:
    1. Each device only.

Yes.  So it should be @error

    2. The oplog keeps applied ops; the error table keeps refused ones.

Agreed.

    3. Through a GET on [@carbon]. An app reads refused ops with a GET such as GET [@carbon]/rejected in the space, and watches it like any other GET. No notification API.

GET [@error]/all
GET [@error]/service/:name
GET [@error]/search

What's interesting here though: normally you can't POST to a local service from a replicated call.  In this case the runtime wires it up, but it means services can't directly POST.  I think that's fine.  They just `throw` if they want to.

    4.  The re-run moves the op to whichever table its new outcome belongs in.

Interesting.  So, I've been feeling as we talk about this that putting the inputs and outputs both in the oplog is awkward.  So far we're saying:

1) Ops store both the inputs, and the local output
2) Ops only go in the oplog once they succeed
3) They only get replicated once they succeed
4) If they don't succeed they go in errors

This doesn't feel correct to me.  As I've mentioned, fundamentally what we're doing is CQRS.  And you've put the command and the query results in the same place.  Instead I would say:

replication messages --> oplog
local changes        --> oplog
                         oplog   --> Services --> SQLite
                         results <-- Services <-- SQLite
oplog response       <-- results
nested GET ops       <--                        tx state
GET ops              <--                        SQLite

E.g., keep the results, potentially with errors, or nested calls, separate from the oplog.

The oplog records "what I received" and the results records "what I did", and "what happened".

State synchronization becomes "find the ops that don't have calls and run them", with some potential rewind/rerun semantics.

Does that make sense?  I'm arguing for separating them, but tell me if I'm solving a problem, or if we'd be better off keeping the two condensed into one.

Just discuss this last item.  Put everything else back in the agenda.

---

    You're solving a real problem: one row holding both the op and its result breaks once ops re-run

The other problem is that most queueing systems record immediately to disk upon receiving new input.  This way if anything crashes they're maximizing their chance that no messages are lost.  I think the oplog should have a similar quality.

    The costs:
    - Two rows per op instead of one.
    - The outbox needs one more filter: publish a local op only once it has an applied result.

I wonder if you could mint a new "result" in memory and save the ID in the oplog op as "result_id".  So that way you have a built-in reference to check what the result was, and you don't have to break your immediate persistence to do it.

    Open questions:
    1. Does the oplog hold only what was received

Yes.

    2. Are the ops still waiting to apply exactly those in the oplog without a result, so the RunQueue is derived from the log?

Yes.  The RunQueue is a running diff between log and results.

    3. Does a local call refused on its own device leave no trace in either table?

I think local calls should always be recorded in results.  Never in oplog.

But: I think what we have discussed and implemented thusfar is that `system` is where all local ops end up.  I would be fine with continuing that direction.  So @carbon -> `system` + `results`

---

    If results are keyed by op hash, the op row can be written immediately and its result added later, with no extra column

Great, do that.

    Only calls on the system handle go to the system log

Ah.  A more recent change.  I had forgotten and you are right.

Um.  So... I'm just trying to remember.  I think we said: Put the device ID on the local ops, which are also marked with @.  So when devices see local ops, they say "Is that for me?" and if not they don't run it.  And that way they can be replicated and visible without causing duplication.  So because you already have them in the oplog consistently, and you also record their results... nothing changes for them, they're treated entirely consistently at the level of op and result recording.

Is that right?

Your third point seems to be rambling on the same subject, I think it's irrelevant if we are straight on the above.


---

    When another device sees a [@carbon] op that isn't its own, it should record a "skipped" result rather than nothing.

Great.  That's perfectly clear.

---

    but I haven't verified it or checked whether rusqlite exposes it.

Can you do that?

---

    The actual rule is that a row's key must come from the op that creates it, never from the rows already in the table.

Ah.  That's more clear, and also makes sense in a distributed system.

    requiring every service table to be WITHOUT ROWID?

What does that mean?  That's different from primary key, right?

    The core would check every table after init()

I like the idea of doing integrity checks after init().

So it sounds like:

1) we enforce compliant schemas
2) we provide op.id()
3) but it's still up to implementors to close the gap

Is there anything else we can do to make the transition easier or more automatic?


---

Hmm... you make a good point about this being an API facet that people will start to build around.  We can't retract it without breaking code in the wild.  Let's drop it and just give the integrity check with a robust error containing guidance on what to change and why.

    Still open:
    1. Should integer ids stay below 2^53 so JavaScript reads them exactly?

Yes.

    3. Is the SQL function named op_id() and registered by every runtime? That's my proposal.

That's fine.

    5. Should the D1 compatibility layer report the drawn key as meta.last_row_id? That's my proposal; it keeps D1 code that reads last_row_id working.

I think this relates to op_id()?  Also fine.

---

    My proposal:
    1. [@carbon] creates carbon_system_identity through its own init(), like any service, so the core no longer creates any service's table.

We agreed that @carbon runs in every space, and puts ops into that space's oplog.  So does carbon_system_identity go into each space?

    2. The core keeps only the op log and results tables, because the RunQueue that reads them is the core's.

Oplog and results are definitely set up per space.  I don't know how it would make any sense for them not to be.

    3. carbon_migration is deleted, since services now track their own migrations.

Sure.

---

    My proposal:
    1. Move system.sql into [@carbon]'s folder, crates/core/src/services/carbon/. [@carbon] keeps applying it when the system database opens, not through init().

Ok.

    2. Give oplog.sql the results table, since the core creates both in every space and in the system database.

I'd rather see a separate results.sql.

    3. Delete space.sql along with carbon_migration.

Why?

---

    we agreed the core reads no service's table

Can't it just GET whatever it wants through a normal op?  What that rule means is to not violate encapsulation.  It doesn't mean you can't talk to the service.

---

    Open question: once the id comes back from POST [identity]/origin, who authors the two ops that run before it answers: [space]/origin and [identity]/origin itself?

Isn't this all running inside a `POST [@carbon]/identity`?  Shouldn't that command be responsibile for wiring everything up?

---

    My proposal: the new device alone. That's honest, because no identity exists yet, and the author's identity field is already optional. Once [identity]/origin returns the id, every later op carries identity id + device.

Uh... yeah you're asking what ID to use when creating an ID.  Are there consequences to this question or is it just window dressing on null?

---

    It will matter once replication exists.

Then you didn't need to say anything in the previous paragraph.

    Another device receiving an op with no identity must check it against its device key alone, and accept that only for origin ops. I've added that as a topic off the critical path.

You're not doing me any favors by suggesting something that's going to screw us over in a minute.

But I also don't see what the problem is here, you're going to need to explain it more.  What's the problem with accepting the device key alone for [identity]/origin ops?  And I think this is true only for identity, not for any other kind of /origin...

---

    nothing has watched the identity space yet, so its refs aren't in memory

I don't understand.

    My proposal: a call that writes refs resolves only after the watch has refreshed.

This feels backwards.  Did you mean to say "Watch should refresh immediately after a call that writes refs?"

---

    start(), createSpace and createIdentity start the refs watch for each space and wait for its first answer.

Does that mean we're blocking while waiting?

    Should every call resolve only after all the watches its commit triggered have refreshed

I lean towards all watches.  Feels like it just gives more consistent behavior.

---

    1. Should this round include splitting the op log from results?

Yes.

    2. Should I build it in three stages with a review between each, as we did before?

No, just build it all at once.  I'm going to bed.

Thanks Claude.

---

    the space keeps a map of replies keyed by op hash, beside the RunQueue

Isn't that the results log that we already agreed you would build?

Or is this a list of "callers" or "receivers" or "waiting"?

  - My proposal: it waits outside the RunQueue, and enters the RunQueue once it's logged at the end of the turn. Then logged goes away, and the RunQueue can hold plain Ops and stop being generic.

This sounds bizarre to have this in RunQueue.  Can you decouple these things, so you have an OpWriter responsible for writing incoming or locally sourced Ops?  And the RunQueue doesn't concern itself with receiving incoming things, it just reads the Oplog, reads the Results, diffs, and determines what it will run?  Single responsibility.

---

    The replies map is not the results

Call it waiting_ops.

    Yes, the OpWriter split works, and the RunQueue gets simpler than you might expect.

Do you still need it at all?  It shouldn't be called a queue if it only has one item in it.

    - RunQueue: the next op is the first one in the op log with no result that isn't already running, and ops apply one at a time.

Is there only one op ever running at a time?  Does that mean this is a serially processed system?  Is that an appropriate design?  At what level do you actually need to serialize application?

    - service
    - space
    - device

Space definitely seems like the safe boundary.  Service, maybe?

    If writing an op's result fails

Why would it fail?

---

    keep "RunQueue" as the name of that query, which is how stories/implementation.md:141 already defines it

Please stop clinging to a story that was written before our current realizations.

I don't like you calling this RunQueue when it's not a queue.

    When a result write fails, should that space stop applying and surface the error?

I mean, is part of the purpose why it's a queue right now so that it can provide this logic safely?

Add to agenda topics: why do service-build and hash exist as separate crates?

---

    The ops with no result yet are the pending ops, which is already the function's name, oplog::pending

Is oplog::pending just calculated directly off SQLite state?  Yes, I'm fine with you dumping RunQueue and leaving that as a calculated value.

    2. When a result write fails, should that space stop applying and surface the error, instead of recording a refusal?

Yes?  It seems like disk errors and such are a pretty critical system failure, they're not related to the op itself failing.

---

    The only thing the space keeps in memory is which op is applying

Can you call it RunState?

     - My proposal: the store classifies SQLite's own failure codes as system failures when they happen, on any statement. These are I/O errors, a full disk, a database that's busy or locked, and corruption.

Yes.

     - My proposal: new calls to that space reject with the stored error until the space reopens. The OpWriter keeps logging ops that arrive,

I would stop even logging ops.  This node is out in this circumstance.  Requires manual resolution of the problem and `resume()` of the space, which will probably need to be surfaced as a UI button.

---

    1. Should RunState be one of three states: Idle, Applying (with the op's hash) or Stopped (with the error)?

Sure.  That's elegant.

    2. Should a system failure stop only the space it hit, with each space resumed on its own?

Yes.

    3. How does the app learn a space has stopped, so it can show the resume button?
       - My proposal: calls to a stopped space reject with 503 (service unavailable), carrying the original error, and the space's watches receive the same error.

Yes.  The app will also want some way of querying or watching the RunState.  Use your discretion.

---

    Open questions:
    1. Should store.rs capture each op's changeset as part of its write transaction, replacing the authorizer's write set?

I'm open to it, but not fully aware of the consequences.  What are the advantages, and what are we giving up?

    2. Should rewind live in its own module, rewind.rs

I agree.

    3. Should the authorizer, scoped statements and op_id() move out of store.rs into a module of their own?

What would you call it?

---

    Using the changeset as the write set

Let's do that.  But can you document the tradeoffs you've mentioned here within implementation.md?

    I'd call it caller.rs

What about CallWindow or CallContext?  That feels more specific and descriptive.  "Caller" is easily confused with the op itself, or could mean a variety of other things.

---

    - An op on a space whose named services aren't all registered is refused at submit, with a 404 naming the missing service, and isn't logged.

That's bizarre and contradictory with the direction and flow of our conversation.  Why wouldn't you log it?

    Stopping a space answers every waiting caller with the 503, not just the caller of the failing op

That's fine.

    - The authorizer now allows PRAGMA table_xinfo on a service's own tables.

What does this jargon mean?

    Holding a session alongside its connection needed a new crate, self_cell

I am trying to get you to eliminate extra crates, not create more.  Don't play whackamole with me Claude.

    When setting up a service has to wait until the applying op finishes, an error from that set-up is still dropped

Add to agenda.

    - The core still has two different Reply types.

Add to agenda.

Talk to me about the extra crate you created, and push the rest into the agenda.  Get rid of the implemented items from the agenda.  Put "createIdentity returns a paper key" in the backlog and get it out of the agenda.  Drop the 403 testing and get it out.

---

    I found one more variance and added it as a topic, "A local service op inside another op bypasses the OpWriter".

Get rid of that topic, we know about that and it's not a problem.

---

    A local call to a service this device hasn't registered can't be stamped with a version, so the op can never apply

Ok.  That makes sense to me.

But I still think you'd be better off allowing it into the log and recording a failure, and then not replicating.  Otherwise, how do you report?  How do you audit?  How do you see a history of these failures?  If you're going to take it out of band of the normal application approach, then you're going to end up reimplementing all of that in a poorer, more janky way.

What if you loosen up the schema and allow the call without the version to exist there?  Or maybe create an explicit placeholder version for this purpose?

---

    Placeholder: a reserved string such as "unregistered". That leaves the API unchanged, but it's a magic value that every reader has to know about

"service missing"

I don't think that every reader needs to know about it, because core is going to prevent it from replicating, and core can't delegate it to any local services, and core is going to mark its result as failed so it doesn't run again.

---

    2. Can the caller wait until the service is registered before it hears the refusal?
    - If the space names the service, the space isn't ready, so this op waits behind the readiness gate like every other pending op.

Yes, if we're still in the middle of initializing it's not "service missing".  You know the service is registered, and you should also know that it's in process initializing.  Do you have some kind of state machine to track the status of the service?  The RunState we just worked on seems appropriate for that.  You previously proposed these states:

    Idle, Applying or Stopped

Why don't we add Initializing?

---

    Proposal: a space's RunState becomes Initializing, carrying the named services not set up here yet, until every one of them is set up. Then it becomes Idle.

Yes.

---

Can you move "services" under "packages"?

Get rid of "examples".

Move "components" and "platforms" under "perspectives/brandon".

---

    - [chat] now has no tests at all. The 16 end-to-end tests in examples/chat-server were the only ones exercising it, and packages/services/chat has none of its own.

Huh?  Ok, can you explain this?  Is there a better way of structuring so that we retain the tests, but group them with the service where they belong?

---

    - The authorizer
    - The session extension

Right.

---

Oh, I see.  Normally ops are replicated, and so we don't need to replicate sub-ops: that would produce exponential expansion.  But local services need to be handled differently, their sub-ops still need to be applied.  So we need a way to replay the replicated ops without replaying the parent op (which contains system calls).

I think the only sensible way to do that is to replicate the sub-ops in this case.  What do you think?

---

    You have it right, and the code already does this.

Ok, so then we can close this topic.

    The two design docs disagree on whether the parent op leaves the device

Then update them.  We've had several conceptions of how this should be implemented: now you know where it landed.

    Should the parent op travel to other devices as an audit entry, or stay local?

Don't mess with it.  It's correct already, and you're attempting to backport an old, wrong notion.

    implementation.md:232 already has a section titled "No secrets in replicated ops"

That's a pedantic and wrong interpretation.

    On this device, a sub-op runs inside the parent's transaction, so it can see whatever the local service wrote before it.

This is the one area where we're screwing ourselves a bit, and might need a second look.  We've broken out of the transactional context by sending out the child logs.

Two possibilities:

A:
1) Instead of replicating the child logs, store the sub-op calls on the parent, complete with their input arguments so you have perfect replay on the peers
2) peers interpret the presence of a "local only, already run on another peer" op as "run the subops in a transaction, but don't run the parent op"
3) this retains our transactional enclosure

B:
1) Store some kind of "parent" reference on all the subops, or a "tx" field for grouping them all in one transaction
2) normal ops have either no parent, or no tx, or a tx that doesn't match any other ops
3) this retains our transactional enclosure

What do you think?

---

    Your "don't replicate until it is done" needs no exception: the publisher already sends only ops whose result applied

Fair.  So we just update local ops with the additional information just before replication.

    - The sub-calls would be filled in within the same transaction that records the op's result, so the log row and the result commit together or not at all.

Good.

    The op hash won't cover the sub-calls.

That's perfectly fine.  We just document that.  The subcalls aren't a part of the call signature, so we don't need them in order to properly key a hash.

    Replay repeats the recorded calls, not the parent's logic.

That's fair...  Do we need to protect these system calls in any way against replays?  Or say that they cannot consume a value that might be subject to rewind/replay?

    The logic at crates/core/src/service_call.rs:212, where a sub-op writes its own log row and result inside the parent's transaction, goes away
  // End a call's savepoint, keeping its writes or rolling them back. An op of
  // its own goes in the log with its result either way.

1) "inside the parent's transaction" - By "transaction" do you really mean oplog/results records?  Your statement here seems to muddy several different concepts.
2) I think what you're saying is that this code was designed for the transactional nesting of sub-calls into the parent context
3) I'm not sure why that goes away, since all other calls that are not local still need it
4) does the code we are implementing for local somehow cover both cases?
5) are you proposing to record sub-calls for all calls, universally, not just local ones?
6) and was that already happening?

    Still open: does every replay of a local op, including rewind on the device that ran it, run its recorded sub-calls the same way?

Yes, and the question feels redundant.  If the answer were "no" then we would be abandoning eventual consistency gaurantees that are fundamental to the whole system we're designing.

We've opened up several subthreads.  Record them to the agenda and just speak to me about one item at a time.

---

    5. No. The list of sub-calls that travels with the op is for local ops only.

And to be clear: it should be RECORDED for local ops only, not just INTEPRETED for local ops only, correct?

    6. Recording sub-calls was already happening for every op, but only as an audit trail on this device.

Right, you mean the results log.  That matches my understanding.

1-4: these all use jargon I'm unfamiliar with, from your internal implementation details.  We don't need to get into that right now.

    Is a local op's sub-call list its own record, separate from that audit tree?

    The audit tree (CallRecord, crates/core/src/service_call.rs:41-56) holds a sub-op's hash

It sounds like you're using three different terms - "audit tree", "CallRecord" and "results" - to describe the same thing.  Am I right about that, and could you consolidate to using "results" more consistently, which is our agreed upon domain language?

---

    Open question: should a local op's sub-call list be kept apart from its result?

This feels obvious/redundant considering decisions we have already made.

We're putting the sub-call list in the same record with the op, in the oplog.  I don't think there's anything ambiguous about that, and I don't know why you're asking about it with regards to the results log.

---

    rewind can't re-run a system call, so it needs no protection there

When you say it can't re-run a system call, I think you mean a "local" call, and that's because we were already preventing that from happening.  But now with the new interpretation of local calls: It doesn't rerun it from the root, but it reruns all the replicated sub-calls.  Maybe that's fine, no conflict?

    The one replay that does re-run system calls is a crash in the middle of a local op.

Agreed, backlog, we don't need to talk about that now.

    My proposal: add one rule for service authors to stories/implementation.md. A local op's sub-call inputs should state what it did, like the ids and keys it created

That doesn't make any sense to me.  I don't see any cause for further restraints on local op writers.  And your language "should state what it did" is confusing and vague, and I think duplicates the records that core is already keeping.  And is therefore redundant and useless.

To conclude: is there any evidence or cause for concern about how the new version of local calls (system or not) will run with regards to replication and replay?  Or can we retire this topic?

---

    My proposal: record only the ones that committed. A refused sub-call changed nothing, so replaying it adds nothing.

That's fair.  The results log still shows the exact outcome of calls.

    Second question: build the replay now or later?

Wait for replication.

Proceed.

---

    The helper always names its tracking table d1_migrations

Yeah, don't do that.  You don't need to copy pasta Cloudflare naming into a context where it's not needed at all.  We decided that CarbonDB core doesn't enforce any particular practices around migrations, remember?  So they don't need to be named consistently on a per service basis.  It sounds like they're already prefixed "carbon_" so if you just name the table "migrations" you're good.

    Open question: adopt the helper for the core's tables now, or wait for a first release and delete dev databases until then?

Adopt the helper, but we will probably still from time to time blow away the definitions and consolidate them until we get to a 1.0 release.

Ready to implement?

---

I would like to review with you the `packages/carbondb/src` code.  I want to ensure that:

1) no state of "CarbonDB" is contained within the JS code: all state should be in rust
2) the JS logic is kept very minimal, it should just proxy to Rust

Update the agenda with any violations of these rules that you find.  Prefix your topics with `JS:`.  For each topic either provide justification for why it needs to be that way, or admission that it should be folded into the core Rust code.

---

    Building it raises two questions:
    1. Who names a service? Two agreements disagree.
       - service.toml holds the service's name (stories/packages.md:21).

Yes.

       - The app names the service when it registers it: carbon.services.add('chat', chat).

Yes.  That's not a conflict, it's just what they get named locally within the space.  The toml name gets overridden at this point.

   The runtime even ignores the service name written in a declared key, for that reason (packages/carbondb/src/routes.js:1-3, README.md:34).

Yes.  So?

    2. How does registration find the file?

You can add a card to the backlog for "map canonical package/crate names onto services".  But I don't think we need that right now.

---

    npm services will run under the core's 200ms default until they get a service.toml

No, we still need to talk about that.  Or to say it clearly: [chat] needs to have a service.toml, and CarbonDB needs to expect it.

---

    - The binding name in a toml only partly takes effect

Why didn't you fix that Claude?  It's obviously wrong.

    - Every call schedules a JS timer, and nothing cancels it.

Again, obviously wrong, and it sounds like you didn't complete your implementation.

    The JS test services' versions cover only their toml.

This doesn't matter right now.

Do you have any (non-dumb) questions before you fix items 1 and 2?  Be concise and don't repeat a bunch of stuff we already know.

---

    The core needs that much from the host because it has no event loop of its own. It runs on Node's JS thread and only acts when something calls into it.

That's an interesting design.  It seems like that would be very memory/CPU efficient.  But is that still going to work when we implement replication?

---

    My proposal: keep setTimeout plus cancellation now, since single-node has no tokio. When replication lands, move timers onto tokio and the crossing

That's fine.  Just put the timer move in the backlog.

    I've added replication's threading to the agenda as an open topic off the critical path

Get rid of that.  We don't need to dive into that further.

    I'll keep db unless you want it renamed to match Otto

No rename, keep it as it is.

Are you ready to implement?


---


Let's review chat.test.js, which is essentially a mark of how well we did in terms of the overall application experience: initializing CarbonDB and a service, binding a watcher, and calling some mutations.

    // An app's start: load CarbonDB, register [chat], and reach the lobby, made on
    // the first start and found again on later ones.
    async function start(config) {
      const carbon = await configure(config)
      carbon.services.add('chat', chat)

Looks good up to this point.

      const known = (await carbon.listIdentities()).some((identity) => identity.slug === 'app')

I think this should probably be:

    const ids = await carbon.call('GET [identity]/identity')
    const known = ids.some((identity) => identity.slug === 'app')

Or similar.

      if (known) {
          return carbon.resumeIdentity('app').ref('personal:app').ref('space:lobby')
      } else {
          const {personal} = await carbon.call('POST [@carbon]/identity')
          return carbon.call('POST [@carbon]/space', {name: 'Lobby', services: ['chat']})
      }
    }

And the last part should use an explicit if/else and should use GET/POST calls instead of relying on direct methods on the handle.  I don't think those direct methods should exist.

I think you maybe created those with some convenience wrapped in in terms of resolving space refs?  But I think it's possible you could build the service calls to be approximately as fluent/ergonomic?  You're just passing around a string ref to the space instead of a memory reference.


---

    - I didn't merge SystemHandle into SpaceHandle (your proposal 4).

That makes sense to me.  What do you think about the [@carbon] calls on system, vs on an allocated space?  Is there also an issue there where calls that are valid on one may not be on the other?

    - Story 02 (adding a device) still uses joinIdentity() and identity.addDevice().

That should use normal .call() as well.  We should discuss what the exact API should be and update the story, but now is not the time to implement it.

    Its join object carries a running two-sided exchange (qr, apply, done), which isn't one call

We have handled most cases like this as [@carbon] calls.

---

    - System only: POST /identity

It's not helpful for you to give example calls which don't specify the service you are calling.

    POST [@carbon]/space on carbon falls through to unknown()

Do you mean on "system"?  Or the "carbon" handle, which is indeed system?  Can we keep our terminology consistent and refer to the space name rather than use "carbon" to mean both space and service?

    My proposal: turn the match into a route table, where each row is (system or space, method, resource, handler). A key that exists in the table but was called on the wrong handle gets an error saying where it is answered

You're then muddying the responsibility separation we set between core and service.

To be clear, I think it is feasible to detect "system" vs "space" and have [carbon] present better messages based on context.  But I don't want to implement this as a general rule affecting all services (look through all service catalogs for similar routes).  Maybe that happens at some phase, but it's later, as a helper/assistant for correcting errors.  But not part of the fundamental routing.

    The alternative I'd rather not take without you: give the system space's calls their own local service name

We have a list of @carbon services in stories/services.md.  Is that up to date?  Feel free to update it to our present understanding.  As an aside remind me: what is the glow command I need in order to print the tables out at terminal width?  Is there some way I can make that default behavior of glow?

    A join has one side waiting while the other scans.

I don't think device joining should be happening in context with system.  We decided to group devices under the "identity" space, remember?  So it may be a @carbon call to facilitate wiring together system calls and replicated calls, but it should be happening on the designated "identity".

---

    Proposal: split the match in crates/core/src/services/carbon/mod.rs:42-63 into two route lookups, one for system and one for other spaces

That sounds really ugly.  Let's look at the table of calls together: I have a feeling we're probably trying to put things into @carbon that may not be a good fit.

    POST [@carbon]/identity        │ The system space          │
    GET [@carbon]/identities       │ The system space          │

These I agree with.

    GET [@carbon]/identity/export  │ The identity space        │

This could be on [identity].

    POST [@carbon]/devices         │ The system space          |

This I think should be done inside the identity space.

So we only have 2 calls that are system only.  What if we make them direct calls on the "carbon" handle, then move the other two as I've suggested.  Now we have 0 calls left on system.

---

Mmm... Ok.  So you're saying that:

1) we need the two identity interactions to be calls.
2) `GET [@carbon]/identity/export` reaches into oplog internals, and therefore needs to be @carbon
3) `POST [@carbon]/devices` also needs to be @carbon because it's dealing with private key stuff that's local only

The same might be true of "joining devices" and "signing messages" when we get that far, because both operations need the private key, and therefore need to operate from @carbon.

So yeah it seems like we should keep the routing in @carbon (the service) but pass it a context so it can determine what other services are operating in the environment, and some sort of tag for "is this system or a space?"  And then the appropriate error messages can be built from within the service.  Does that sound right?

---

Commit:
standardize application boot and drop handle methods in favor of service calls

---

Let's update the README and stories.  In "Status" I would like you to spell out that:

- we have an initial proposal for what services look like (https://github.com/TorchlightSoftware/carbondb/blob/main/packages/services/chat/index.js)
- and what the app code should look like (https://github.com/TorchlightSoftware/carbondb/blob/main/packages/services/chat/test/chat.test.js#L20-L33)
- core API is implemented in single node mode
- ready for building replication

I'd like it if you could both put examples from the links I posted, as well as link to those files within the codebase.

In addition, I'd like you to update all stories, architecture and plans to be accurate per the present state of the code and the decisions that entails.

Questions before you begin?

---

    How much of index.js to quote

Just one of the service definitions.  `POST [chat]/messages` is sufficient.

Other than that you're good.  Proceed.

---

    the docs promise that an app can run its own [identity] service, and the code refuses it. Several docs say it: stories/README.md, stories/implementation.md, and the FooCorp example in stories/oplog-examples.md. But registering a service under a name CarbonDB already uses is refused with a 409

That's going to be a problem.  I should be able to have:

system
 |_ identity:Brandon [identity (carbon)], [space], [device]
     |_ company:FooCorp [identity (Otto)], [space], [books], [automation]

Services are namespaced on a per-space basis.  If there's no other [identity] registered in that space, it's perfectly valid for apps to register their own.

---

    The root cause is that the core keeps one name→service map per device, not per space

That makes sense.

    The space's origin records which package each name runs

That sounds fantastic actually.  That's exactly the right place for a "manifest" of what the space requires.  I definitely think that should be our destination.  But we don't necessarilly need to implement it right now.  Agreed on updating packages.md and package-names.md.  Are you ready to do that?

---

- crates.io: carbondb, carbondb-core, carbondb-service-bindings, carbondb-space, carbondb-identity, carbondb-device

Mostly makes sense.

1) how is carbondb-core different from carbondb?  Why would I choose one over the other?
2) carbondb-svc-space <= prefix services with svc
3) I know we're targetting WASM builds for the builtin services, but they will also get compiled as blobs into the binary... therefore they probably don't deserve their own dedicated crates?

- npm: @tlsft/carbondb, @tlsft/chat-service, and unscoped carbondb. We already own the @tlsft scope, since @tlsft/service-key is published there.

I think unscoped `carbondb` is the main target.  I don't see any urgency or reason to grab the @tlsft scoped one: no one else can grab it.  @tlsft/carbondb-svc-chat would be the preferred name for chat.

    Publish the real code as 0.1.0, not empty placeholders. As far as I recall, crates.io's usage policy bans name-squatting and lets them remove crates that are only placeholders. Real code avoids that risk.

We have real code Claude.  Quit being a dumbass.

---

    Eleven files reference the old names, including README.md, the stories and the chat test

Those should be renamed.

    Do you want carbondb-service-bindings renamed too

No.  That's not a service, it's service bindings.  Keep the name as is.

Proceed.  Get everything ready to publish, and then make a one time throw away script that will publish them all when I run it.

---

    carbondb-node stays unpublished

Is that used internally in the package that's published to NPM?  So doesn't need to be published bceause it's intermediary?

    carbondb on npm still contains only the Linux binary

Elaborate?

---

    Checking loginRedirect belongs in identity, against a trust list we already keep

Yes, sure.  I was only saying candle-connect gives you the service registry to do it with.  Although maybe we should rely on the service registration process instead?  Where the public keys of the services are registered?  That seems like a stronger cryptographic gaurantee than the service map itself.

    Yes, what I built breaks the intended flow.

Yeah, don't do that.  You don't have the authority to arbitrarilly break working functionality because you decided it was for the best.

    For a deleted tenant I'd answer 401 with its own error code, TENANT_NOT_FOUND

Shouldn't that be a 404?  You literally put NOT_FOUND in the code.  It just needs proper metadata on it so it can be detected distinctly.  Tell me if I'm wrong about this... 401 just doesn't feel like it describes the situation.

Oh, but also why does ANYTHING retry infinitely?  It could retry a few times, or with an exponential backoff loop.

1: agree with what you're saying but it's pretty verbose.  I don't need to get into the weeds with you here.

2: I disagree with your decision, let's fix it

3: A missing login domain is the same kind of problem — is it the same answer?

Yes. But...

    issueToken also fails with 500 when the tenant has no tenant_domain row

That's not the error code you should be returning for that.  500 means something blew up.  Something didn't blow up.  That tenant just doesn't exist.  404 is what I have my money on, but you could also argue for 401.  And TENANT_NOT_FOUND should be the error type for them both.

    getTenantLoginUrl takes the first tenant_domain row, in no fixed order. Torchlight has many rows, so tenantLoginUrl could be any of its service hosts, and that's where DataAccess sends someone whose session is over

That's wrong.  I think login expiration/redirect to the same app was working before, and may have been modified adversely after the fact.  Regardless, I don't need you to do a historical dive on that, I just need you to fix it.

    @tlsft/service-error and candle-env's ApiError have the same shape. Worth merging

Yes, do that too.  That's annoying that you did not do that when service-error was written.

Update the agenda, adding any new topics that are necessary.  Then address only one with me.

---

    For deciding where a session may be sent, service registration adds nothing over the service map

Ok.  I'll concede this, and I actually don't think it's important what we do right now as long as it doesn't leave the system completely open to fraud.

    My proposal: use the service map plus the tenant's tenant_domain rows

Yes.  So tenant_domain could potentially override the service map.  I think that complies with our actual current semantics and would involve reusing the existing logic, yes?

---

    1. One function in candle-service decides who owns an origin: the platform, a specific tenant, or nobody. Three checks use it:
       - CORS: any owner.
       - Handoff sender: any owner.
       - Handoff destination: the platform, or the session's own tenant.

That's clear, and I agree with that.

    2. POST [identity]/person/auth refuses a loginRedirect outside that set.

Yes.

    3. GET [identity]/person/launcher checks a loginRedirect it is given.

Yes.


---

    It's agreed that a missing tenant answers 404 with code TENANT_NOT_FOUND

Yes.

    Does the client treat every 4xx answer from renewal as "session over", except 429 and 408? Then the 404 ends the session without the client checking for that code.

You need to be more explicit about the consequences of that in terms of the specific use cases we've laid out.  You're waffling all over in terms of the way you're presenting this.  But I think the answer is yes.

    If identity is unreachable, the client retries a few times with backoff, then shows Try again.

Yes.

    - Do you agree it should never send the person to login during an outage?

What problem are you trying to solve?  Does it result in improved user experience?

Be concise in your response.  Let's wrap this up.

---

┌────────────────────────────┬──────────────────┬───────────────────┐
│            Case            │      Today       │       After       │
├────────────────────────────┼──────────────────┼───────────────────┤
│ Tenant deleted (404        │ Stuck on "Can't  │ Sent to the login │
│ TENANT_NOT_FOUND)          │ reach sign-in"   │  page             │
│                            │ forever          │                   │
├────────────────────────────┼──────────────────┼───────────────────┤
│ Tenant lost its login      │ Stuck the same   │ Sent to the login │
│ domain (404                │ way              │  page             │
│ TENANT_NOT_FOUND)          │                  │                   │

You can't send to "the login page" because there is no "login page" defined if there is no tenant record.

You need to display an error; whatever the user formatted version of TENANT_NOT_FOUND is.

│ Session revoked or         │ Sent to the      │                   │
│ expired, person deleted    │ login page       │ Unchanged         │
│ (401)                      │                  │                   │

Fine.

├────────────────────────────┼──────────────────┼───────────────────┤
│ Garbled token (400)        │ Stuck            │ Sent to the login │
│                            │                  │  page             │
├────────────────────────────┼──────────────────┼───────────────────┤

How do you know what tenant this is for, or what login page to send them to if the token is garbled?

│ Rate limited (429) or      │ Retry            │ Unchanged: retry  │
│ request timeout (408)      │                  │                   │
├────────────────────────────┼──────────────────┼───────────────────┤
│ Identity down (5xx or      │ "Can't reach     │ Retries with      │
│ network)                   │ sign-in" at once │ backoff, then Try │
│                            │                  │  again            │
└────────────────────────────┴──────────────────┴───────────────────┘

These should all have the same behavior, with the exception that if you're getting rate limited, then you should start at the bottom end of the exponential backoff.

Is this clear?  Do you know what to do now?

---

    This organization's account no longer exists. Contact your administrator.

No.

    Your account cannot be found.  Please contact your administrator.

2, 3, 4: fine

    does "the bottom end" of the backoff mean its shortest delay or its longest?

Does a bear shit in the woods?

Ok, please implement all 5, including consolidating the errors to use service-error.

---

     0.8 no longer lets us choose a space's id

Ok.  I'm not sure we needed to do that to begin with.

    its high-level Space API won't let a device write through its identity group

Meaning that it will write as the device?  I'm not sure that I care... yes it's an implementation detail that changes, but we should still be able to map our own identity abstraction over the device/space semantics.

    I recommend building rewind first, with no network involved, then wiring in p2panda

That's fine by me.

    1. Should a space's id be p2panda's id

Sure.

    2. Should a device write through its identity's group, or be added to each space directly?

We need to add the devices directly.  However, you need to understand that:

- CarbonDB's API should encapsulate this
- CarbonDB should operate in terms of identity
- under the hood we will wire up devices
- an identity should have the capacity to "tag" its devices
- a service should "tag" its operational requirements
- an identity should provide an assignment policy which looks at the tags on `devices` and `services inside a space` and assigns which devices should hold [pull, read, write, manage]
- carbondb core could provide a default implementation of this policy
- we could backlog for now the ability for individual identities to override it (need to understand use cases better, which we will only get from testing)
- this works similarly to the way Kubernetes assigns pods to available nodes in accordance with tags on each.  Don't do a deep dive into Kubernetes or stretch this metaphor too far, but both involve some sense of "jobs" being allocated to the appropriate "hardware"

    Our design says "an identity is a p2panda group of its devices" (stories/implementation.md:224). That model is blocked

No, it's not blocked.  The way you need to implement it has changed.

    file the bug upstream

You can backlog that.  I would like to build a sensible implementation of CarbonDB first, so that we can share more about our use case if and when we request any changes.

    3. Should we build rewind before any network code?

Sure.

Please create the agenda with the goal of implementing sync using p2panda.

---

    Open questions (3 here; 18 more across the agenda):
    1. Where is each op's changeset and read set kept? Beside its result (which a rewind replaces), or somewhere else?

I don't understand why you're asking this.  The changeset and readset come from the SQLite extensions that we're applying.  If I remember correctly we need to store the readset to understand what state was "observed" when the Op occurred, which helps us to build CRDT semantics.  And the changeset gives us our actual database mutations to apply or rewind.  Is that accurate?

It seems like you might need the readset on the Op, because doesn't it need to be replicated in order to power the CRDT semantics?  And the changeset feels more appropriate as part of the result.  But please tell me if this is making sense.

    2. How does an op from another device enter the op log? The OpWriter is meant to take synced ops, but no call delivers one from outside yet.

I think this is obvious given the context; we're using p2panda spaces to replicate the Ops.  The Op is just a message in a p2panda space.  Is there something non-obvious in your question that I'm missing?

    3. How is a hybrid-logical-clock time stored in the op's time field so that the existing sort (time, then device, then op hash) still orders ops correctly?

Again... it sounds like you should have an obvious answer to this question.  Please try, and let me know if you have an actual difficulty or conflict.

Please just talk to me about item 1.  Set 2 and 3 aside as separate topics.

---

    - It isn't "what state the author observed". It's the tables and columns a handler read, which the SQLite authorizer reports

It sounds like you're splitting hairs with me, and what you just described is 99% the same.

    When a late op lands, rewind re-runs the later ops that read what the late op wrote

Ok... and the way it determines that they are "late" is because they are observing earlier state than more recent local ops.  Right?  Or area you able to determine that independently from a clock?

    Why I'd keep it off the op:
    - Redundant
    - Wrong after a rewind
    - Untrusted

Ok, valid.  Seems like you're heading in the right direction but I'd still like clarity on the above.  And this needs to be documented precisely by the way.  Other people will have questions about how the replication works.

    Our design deliberately left that out: ops relate through the clock, not through lists of other ops

Ok, that probably answers my earlier question.

    p2panda tracks causal dependencies internally, but we don't use them

It's a little bit strange to me that we're discarding both `p2panda` and `authorizer` tracking in preference for our own method.  In particular I would lean towards using p2panda's existing mechanism unless we have a good reason for inventing our own.


---

    - The design says a late op that doesn't overlap any later op applies in place. Otherwise only the overlapping later ops are undone and re-run (stories/implementation.md:189-190).
    - But the late op's read set and changeset only exist once it has run.
    - Running it on today's state gives the wrong answer. That state already includes later ops the late op should never see.

It sounds like you can't apply an op in place without causing a cascade of reapplication of ops afterwards.  I don't understand.  Can you show me an example?


---

    No: a late op can almost always be applied in place with no cascade. I overstated the problem last time.  A rewind happens only when the late op actually conflicts with something after it.

Ok.  I didn't review your examples, because the way you're framing this seems more clear now.

    When the check fails, how far back do we undo?
    - Option A, every later op:
    - Option B, only the overlapping ops:

How about C, only ops from the affected service?  State isolation is achieved within service boundaries.  The services follow "actor model" rules.

I don't like the idea of B: that sounds like it could lead to some really nasty edge cases based on an innocuous programmer putting some if statements in a service call.

Of course, services can call each other, and can call out to the web at large.  Do we need to tag side effects so they can be prevented during replay?  Do we need to memoize outside responses so they don't give inconsistent results with the replay?  Is it acceptable that consistency is only gauranteed at the single-service boundary?

---

    the design already keeps outside calls out of replicated handlers

Hmm... describe that further.

    a single op can span services, because a call runs inside the calling op's transaction. So "affected" has to grow.

So you're proposing to basically track transactional state, ensure consistency across services.  Sounds feasible from what you're describing but let me see where this goes.

    before the late op runs at its true place, we must already know every service it might call

In the Otto framework, when the UI code instantiates an API client through candle-connect, it registers the service calls that it will be making.  This serves as a sort of pre-flight authorization check: if the user's JWT doesn't have access then the Javascript knows that this UI route is forbidden; it doesn't need to query the backend to find out.

We could require something similar for the service-to-service calls.  They could declare their subservice calls within the route definition.  This would determine what keys get registered.  And anything not registered would be denied.

This decision could percolate into the CarbonDB service definitions.

Although; I must say that this would be a transgression from our previous domain boundaries.  We have been saying up to this point that CarbonDB doesn't know anything about the internal implementation of services: the routes they support, their input/output schemas, etc.  Requiring services to provide a declaration of the service-to-service dependency tree would cross that boundary.

Maybe for this reason it would be better to detect it through the observed call tree rather than requiring an explicit registry.

---

    I agree with you that the observed call tree is better than a registry. It works, but only if rewind is allowed to retry

I'm fine with that.  We should document it, and if at some point we need to change these semantics we can revisit.

    What is only implied: that a replicated handler must not call the web.

It doesn't just break determinism, it reveals something deeper: that by replicating a call that does "GET google.com" you would be calling that N times.  We need a deeper way of separating [dependencies, side effects] from [pure function, SQLite state mutations].  The former are outside our replication model's concerns, capabilities, and responsibilities.

This may be why Holochain's "Zome" semantics feel so bizarre coming from a REST API developer perspective: they had to separate state operations into a different layer from side effects and dependencies.

    The pattern for outside calls:
    - A local service such as [@weather] runs once, on one device, fetches the forecast, then calls POST [garden]/forecast {temp}.

I see what you're doing in terms of trying to work with our existing model.  And placing "dependencies and side effects that are outside our replication model" into @-prefixed services is an idea I'm willing to try on.

Where I would disagree: Services need to be defined in a way where they can function somewhat normally.  In node.js, it's natural to call `fetch()`.  You could maybe make the argument that "this environment is special, we need you to call GET [@web]/fetch instead of fetch()" and get away with that.  But I don't think you're going to get away with inverting the entire call paradigm, and saying services can never call the web.  That cripples the service environment to the degree that no one would want to use our software.

So at most I would consider bundling dependencies and side effects with local calls like `GET [@web]/fetch`, and incorporating some kind of semantics where it's possible to call them from normal services (run once, memoized result in the Op).

But it's also worth considering whether even that move is too much friction - you're literally saying that anything with side effects or dependencies must be rewritten into a local service call, and that's going to massively disrupt someone's ability to use existing ecosystems like NPM.

Something else worth considering: we want to shoot for WASM services.  So maybe the right call is to simply let the WASM environment be the place where this is enforced, measured, and reasoned about.

Let's just talk about this, and set other agenda items aside for now.

---

    Your "run once, memoize the result in the op" idea is the right model. It works because an outside response can be treated as more input from the author's device, and replication already carries input. WASM is where it can be enforced completely; Node can only do it best-effort. None of this blocks rewind on a single device, so I recorded it as an open question rather than something to build now.

Thanks.  That sounds clear.

    The model: outside responses are author input

Agreed, but I think we have significant challenges to doing this outside a WASM environment.

    Why WASM is the place to enforce it

    - WASM: a module reaches the outside world only through host imports. The host sees every call, so recording and replay are complete.
    - Node: CarbonDB can wrap fetch for each handler. But npm packages that use http, net or sockets directly escape the wrapper.

Yes, exactly.  I think we will find ourselves moving away from non WASM services very shortly because of this.

    Limits we'd have to accept

    1. A rewind can change which outside calls a handler makes. Replicas can't fetch something new, so an op whose calls no longer match its recording would be refused identically everywhere.

Well, you can just call the outside system exactly once.  If the inputs are different, then the memoization doesn't apply, and you need to call it.  Which means we need a way of providing only-once semantics for local calls, even when they are triggered through replication.  But I think it's possible for us to do that deterministically.

    2. Side effects such as POST or email happen once, on the first run. If the op is later refused after merge, the effect can't be undone.

Sure.  This is the unavoidable and obvious consequence of being connected to the real world.

    3. Awaiting the network blocks the space.

Create a topic for that and set it aside.

    4. A recorded response is trusted the way the author's input is.

Record this topic as dropped.  All of these transactions involve cryptographic signatures.  We know who made the claim, and we can layer on fraud detection and reputation.  But that is so far down the road right now, not anything I want even in the backlog.

So to summarize:

1) Should we add WASM implementation of the builtins to our immediate target alongside the rewind/replay?
2) Should @carbon also be WASM, or we just don't care/trust it because we implemented it ourselves.
3) Should we/can we support NPM modules at all?  What do we do with [chat]?  Do we include it as a Javascript example, but compile it to WASM too?

Respond only to these three questions and just quietly update the agenda with anything else you need to.

---

    build rewind first, then the WASM host, before any p2panda work.

That's different, but not in a way that I care about.  WASM is still coming before p2panda, so you agree with me that far.

    The builtins gain nothing from WASM for the outside-world problem.

No, but we intended to implement them as WASM anyway, for other reasons, and it would give us some examples and let us validate the WASM runtime.

    Doing both at once doubles the moving parts in the apply path

I didn't literally mean at the same time.

    My recommendation: yes, but only as a tier for services whose handlers just read and write their own SQLite.

That's not useful Claude.  You just took an intention - to convert Otto services to CarbonDB - and rendered it so impotent as to be meaningless.

    [chat] already fits that tier

That's fine, but it's literally an arbitrarilly simplistic example app right now.  It's not the measure of what is possible.

    A JS service compiled to WASM can still bundle pure-JS npm libraries. The only libraries that break are those needing Node built-ins like http, net and fs.

Yes.  I'm just saying that the WASM compile tools handle that for us, rather than us implementing our own detection/interpretation system based on a target for which it's not feasible to accomplish the goal.

I think I would lean towards not supporting NPM/Node, but requiring that everything be converted to WASM.  We need a way of analyzing, approving, and shimming the dependencies and side effects with single-run and memoization gaurantees, and it doesn't make sense to allow an NPM module into the system which very easily could result in spamming duplicate reads and writes out to systems that are unprepared for that.

Ok, so to recap: You can build rewind first, and then WASM, but put both on the critical path.

And let's drop NPM as a viable target, unless you have a good argument for why not.

---

    My proposal:
    - Rust services: the builtins compile as components for the wasm32-wasip2 target.
    - JavaScript services: compile with ComponentizeJS. [chat] would be the first.

Sure, let's try it.

    - One typed interface for every language, instead of an encoding we'd maintain ourselves.

Not sure what you mean by this.

    - WASI's standard HTTP interface later becomes the single point where outside calls are recorded

Yes.

    - Cost: large modules for JS, because ComponentizeJS embeds a JS engine (StarlingMonkey) in each one.

Is there a better alternative?

    The alternative: plain WASM modules with our own JSON-in, JSON-out encoding, and Javy (QuickJS) for JS. Modules are smaller, but we'd maintain the encoding for every language, and there'd be no standard hook for outside calls.

I like what sounds like small modules and a simpler implementation with less moving parts.  Are there disadvantages in terms of portability of code?

    call doesn't need async in either option.

I don't follow your logic on that at all.  You're going to take services which could have arbitrary IO inside of them, and force them to be synchronous?  I don't think you're anywhere near hitting the target.

---

    Refactor opportunity: once the core is async, callback chains like crates/core/src/services/carbon/steps.rs:706-739 become straight-line code

That file is only 80 lines long.  Say what?

    WIT is a single file that describes the contract between the host and a service

Is that a part of Componentize?  Or WASM components?

    With plain modules, we'd hand-write that glue for every language ourselves

We're obviously not going to do that; it's way outside scope.

    Is there something smaller than ComponentizeJS?
    Yes, probably: componentize-qjs

What makes it so much smaller?  Does the author comment anywhere on their intentions, why they wrote it, how they themselves intend to use it?

    ComponentizeJS is mature, but its imports can only be synchronous

That might totally kill ComponentizeJS for us right there.

---

    - Who: Tomasz Andrzejak, a Microsoft engineer. He's also a regular contributor to StarlingMonkey (33 commits) and jco (82).

That's good enough for me.  At the very least it seems like if it's retired there will be commentary and alternatives proposed.  And he can probably be relied upon to maintain an API that's not fundamentally different from the original.

    No fetch and no console built in

That seems doable, right?

    Async imports are still settling.

Mmm... I think we should go down this implementation path, but add a docs/wasm-services.md with things to watch in terms of how the ecosystem evolves.

    Your call: try componentize-qjs through jco for [chat]

Yes.

    , and accept writing a fetch over WASI HTTP when outside calls arrive?

Implement it and test it along with our WASM implementation.

---

    Should the core move off Node's JS thread?

Yes.

    - Async components need something to drive them, and later so does p2panda's tokio runtime.

Yes.  And it makes the core more focused and fully implemented in Rust, with JS as a thin wrapper, as intended.

What else do we need to discuss?

---

You also need topics for:

- Update existing builtin services to use the new WASM format
- Include the builtin services in the binary

    The op's author device owns its outside calls. On a re-run, a call whose inputs aren't in the recording is made only by the author's device.

What happens if the author device is offline when the re-run is occuring?  There must be a device that triggered the re-run chain in the first place, and that device must be online.  It seems like we could make that device responsible for the call.

I think this would give us more reliable oplog semantics.  But would also necessitate that these calls do not rely upon which node they are fired from.  Which may not be a constraint we are willing to accept.

    - The response travels on the op's log row, keyed by the call's inputs. That's the same way a local op already carries its sub-calls.

Double checking: we're preserving the atomicity of Ops, correct?  sub-calls were embedded in the parent call, and now IO dependencies and side effects will be embedded.

    - Other replicas that need that response pause the space, following the timeout precedent, and resume when it arrives.

This is a weird thing to say.  The op is atomic, yes?  You shouldn't be awaiting some other state in order to apply an op.

---

    It also means a re-run can't make a new outside call in the middle of replay

Why not?  I feel like we're going around in circles here.  But I don't see why the atomicity of an op is contradictory with: you replay it, it now sends a different GET request, which therefore must be rerun.

    My new proposal is that the op is refused on every replica, and its author re-issues it.

This breaks fundamental expectations that users have: they don't expect something that went through to retroactively be refused.  And that really defeats the whole purpose of using CRDTs: you can't just say we made everything consistent: by destroying the data we didn't like.

I think what we're most concerned about is scenarios cause "eventual consistency" to result in multiple calls, even after memoization is applied.

    Why "the device that triggered the re-run" doesn't work

I mean the jerk that was sitting offline and originated the Op that triggered the replay on other peers.  Make that guy responsible for new calls.  In git, if you try to push and the main has moved on, it makes you pull first.  In that sense the peer wanting to rejoin could be responsible for catching up.

I suppose this could also result in multiple replays: who's to say there aren't multiple people sitting offline, and when they come back online each of them will have to replay side effects. (I'm just going to call them side effects from now on.)

---

"Rolling wave rejoin": Can a rejoining node first publish a REJOIN op, then pull, then catch up, then complete the REJOIN?  That way other peers would be aware that someone else is behind, and could wait for state resolution on the peer before initiating their own sync...

To criticize my own idea, seems a bit clever, but maybe naively doing things that we shouldn't have to with CRDTs?  I think we shouldn't need to coordinate or serialize our rejoining...

    You're right, and atomicity isn't the obstacle

Hmm.  But it also seems that whoever reruns the side effect needs to publish a new op.  Maybe the right answer is to deny the old op initially as you say, but then support an auto-resolution where the original device (or any other, as long as it's deterministic) can run the side effect and issue a new op, pointing at the old.

Problems:

1) violates atomicity of ops
2) could still cause amplification effects, or be unstable in potentially "normal" operating conditions

How does Holochain deal with side effects?

---

    How Holochain handles side effects

    - The author's code runs once, on the author's own node. Its result is data: entries committed to the author's chain.
    - Other peers never re-run that code. They only run validation over the data, and validation must be deterministic. It can't read the time, randomness, the node's own state or other cells (validation docs (https://developer.holochain.org/build/validation/)).
    - Side effects go in post_commit, which runs on the author's node after the commit, outside its transaction (zome functions (https://developer.holochain.org/build/zome-functions/)).
    - Outbound HTTP: I didn't find a host function for it. As far as I can tell, apps make external calls outside the DNA, but I haven't confirmed that.

---

    How Holochain handles side effects

Roger that.  So you replicate database state, not the entire application state.  I think this gives us a clue as to where we're going astray here: we're drawing the box too wide about what CarbonDB assumes responsibility for.

I think what that would point us towards recording the database changes in the op, and then replicating those database changes rather than replaying the service.  This gives CarbonDB a more focused scope, which I like.

    replay what a handler wrote, not the handler itself

Yes, although I disagree on some of the details...

    - Scope: only ops whose handler made side effects. Pure ops are still re-run, exactly as agreed.

There's no point in making this distinction.
- A POST with no database mutations gets recorded as a no-op (still may have returned valuable data from side effects)
- should we include output in the op, not in a separate results table?

    - What the op's row carries: its recorded responses and its changeset.
    - On a rewind: replicas re-apply that changeset instead of re-running the handler.

Sure.

    A pure validation hook can check the result against the new state.

We don't have validations in the same way that Holochain does, and I don't know that I want them.  We have input/output validations (serve a different purpose) and Otto specifically has database schemas and perhaps some additional validations that get applied over top in Javascript.  So in some sense you could apply schema validations to this purpose.  But I don't think either of those get at what you would really want.

You would really want a validation that looks at the op inputs, and validates that the database ops which occurred are a plausible result of the op (fall within its expected parameters).  To write that code independently of the implementation itself is... I think a bit nutty and redundant, and speaks to why I don't use Holochain.

I think at some point, you're going to great lengths to rerun the service and... maybe you should just rerun the service.

Let's say the side effect we're talking about is:

    GET google.com?s=foo

And the replay changes it to:

    GET google.com?s=bar

The Holochain approach would say that it doesn't matter: you may reorder the database ops, but you're going to retain the original search as it was run.

Wouldn't we accomplish the same thing with our WASM sandboxing and memoizing side effects?  But with more deterministic and consistent internal application state?

    - What it costs:
      - the handler's business rules aren't re-run against the new state;

Which is a problem because they don't get applied at all unless you rewrite them in a pure function validator format ala Holochain.  I'm trying to shoot for an application writing experience that feels less foreign.

I see another downside to this approach: as a result of this change, the state of peers can now diverge significantly due to any accidental storage of state.  If I make disk access, or even create some state in memory, then my original peer who ran the code now has different state than all the other peers.  Running the handlers on each peer has its own dangers, but might be less of a footgun than local state getting lost entirely.

So... I lean towards:

1) keeping our originally intended semantics and run conditions for services
2) leveraging the WASM boundary for awareness and control of side effects
3) side effects only apply on initial run, never on replays
4) perhaps side effects can be tagged as safe to rerun, such as a cache?

---

    Your ?s=foo / ?s=bar example is exactly how memoizing works

Well no, it's not how memoization works in the traditional sense.  You only get the same result if you provide the same input.  I'm saying: you may change what you want to search google with, but this system will NOT re-query google.  It will leave the database holding the answer of the old search result:

    DB: topic: ducks
    GET ?s=ducks
    DB: result: flapping winged creatures

Replay:

    DB: topic: porcupines
    GET ?s=ducks
    DB: result: flapping winged creatures

This could be default behavior.  Maybe you can configure the side effect calls you're binding to WASM to set different policy though.

    - Disk: a component gets no filesystem unless the host grants one
    - Memory: Giving each call a fresh instance closes that leak.

Sure but does it also close down application development in a way that feels restrictive, and will make it difficult to port existing apps?

    On tagging some side effects as safe to repeat

    It only works for effects whose response never feeds the database or the output,

Hmm.

    Open questions on "Replicated handlers reach the outside world through recorded calls"

Can you rename that to "Replicated handlers' side effects are mediated by WASM boundary"?

    1. How is a replay's call matched to the recording?

I don't know what problem you're trying to solve.  You're not using any terms that we've agreed upon.

    2. Should each call get a fresh component instance with no filesystem? That would stop handlers keeping state outside the database.

Feels very limiting and restrictive to impose this on all application developers.

    3. Should we allow a "safe to repeat" tag? My lean is to wait.

This needs test scenarios attached to it in order to sensibly discuss it.  But set it aside as its own topic, we don't need to discuss now.

---

    Module-level caches and set-up state, which existing apps rely on, would reset on every call, so porting would get harder

Mmm... on the other hand the existing Otto services are meant to run on Cloudflare workers, which also has similar restrictions.  You can't expect it to stay running between calls, and you have to declare all your side effects as bindings in wrangler.toml.

So I'm just saying: I don't want to deny these things, but they must be declared and mediated by the WASM boundary.

    Moved the safe-to-repeat tag to its own topic, off the critical path

That's kind of annoying, feels like you're playing whack-a-mole with me.

    - My proposal: the call fails the way an unreachable network would, identically on every replica. The handler can catch that failure like any network error.

I think we might need to do that for now.

But:

1) we need to document and extensively test these scenarios
2) should have a table explaining consequences in different scenarios
3) I don't think you can just push safe-to-repeat under the rug, that's very relevant and intertwined; it's how the application developer answers "how do I get the behavior that I want from the WASM bindings"

---

    A received op's time is completely unchecked.

Can't you just fix by rejecting as out of bounds?

    Every op now computes its changeset twice

What?  Why?  I don't remember agreeing to that.

    Changesets are stored as hex text, because the core's Db binding only binds JSON values.

Do you have a suggestion for something better, or do you think that's fine?

     - Concurrent inserts into one table always rewind, because the append tag isn't built yet.

Can you build it?

    A replica assumes a space's origin arrives first

That's fine.

---

    An insert can fail on a taken key, a unique value or a missing foreign-key parent

That's something well known that we've discussed before.  It's why services must be provided with an ID generator function (implemented by core and passed through the WASM boundary), and the schema constraints you are mentioning should be a part of the documentation for service writers.

    Comparing writes by row only makes sense once this is fixed

So... what are you waiting on?

    how should a write's dependence on existing rows count as a read?

Can you explain with an example?  You're using a whole bunch of jargon.

---

    An edit or delete looks up its row first, and reads are only known per table, so a late insert still rewinds any later edit to the same table.  Recording reads per row, or the append tag, would close that gap

Ok, and I think I already approved and in fact asked you to implement recording reads per row.  So what's holding you back on that?

---

    I found only your "writes by row is fine for now"

Yes Claude, that's a clear approval, is it not?  I expected you to act on that.

    When a query looks up one row by its full key, the core records a read of that row instead of the whole table.  Anything else (joins, subqueries, WHERE text = ?) stays a read of the whole table.

Agreed.  Should be documented for service authors.

My only concern: is parsing SQL a reliable way of getting this information?

---

    Parsing alone isn't reliable, but it can be made safe. The parse may only narrow a read from the whole table to one row. Anything it doesn't fully recognise stays a read of the whole table

Ok.  Safe is good, and you're saying the consequence of a failure is performance, not incorrect behavior.

    Every service statement goes through sqlparser for the table-name rewrite

Yeah... let's just be sure we document every instance.  This might be the target of a refactor later on if we determine a better method.

    Where SQLite would read more rows than the query text shows, and the check for each

Write tests for all conditions.

    A second witness, used in tests only: SQLite's query plan states its access path, e.g. SEARCH note USING PRIMARY KEY (id=?).

Can you dejargonize this and explain what you mean?  By second witness do you mean another SQLite connection open in parallel?

---

    Reads are recorded per row

Should be on critical path.  It's literally the thing that we're about to do next, and I'm requiring you to do it before we proceed with WASM implementation.

    - What the planner can vouch for is reach: that a given query reads one row by key.

Would it therefore be a better implementation of the "row reads" rather than parsing SQL?

---

    Each statement an op runs is compiled twice

Can you cache this, storing a shorthand version of only the attributes we care about?  Maybe as a tag system, or a short JSON metadata field?  Then we only have to ask for the plan again on queries that we've never seen before.

---

    It lives in memory, one cache per space, not on disk. After a restart each statement is planned once more, which is cheap. A stored copy would have to be invalidated when SQLite is upgraded or the schema changes, and the saving isn't worth that.

I completely agree.

    only the second compile, the one that asks for the plan, goes away

Cool.

Consolidate this into the "Reads are recorded by row" topic, and get rid of topic 7.

---

implement per-row reads

---

    New crates/core/src/statements.rs: a per-space, in-memory cache keyed by statement text, cleared whenever SQLite's schema version changes

Nowhere were you authorized to create a new crate.  Is this just an internal organization mechanism, not required to be published?

    The first time a statement is seen costs about six small schema queries and the plan.

That seems like a lot.  Can you explain?  Maybe it's nothing in terms of performance cost though.

---

    Let the planner do the structural checks.

I would do that.  But skip the other proposals you've made.

I would like this code to live somewhere related to its function... and properly named.  What would you call it, a query analyzer?

Basically everything related to reading queries, figuring out what their scope is, and translating that into semantics that we care about, should be encapsulated somewhere.  That should either be a single file, or a directory containing a few files.

---

    Proposed shape:

    query/mod.rs
    query/namespace.rs
    query/lookup.rs
    query/authorizer.rs

I like, mostly.  These things belong together for sure.  Not sure about the name "query".  Are you saying most of statements.rs would go into mod.rs?

    What stays out:
    - call_window.r

Agreed.  Should "op" related stuff go into its own folder?

    - store.rs

Should "write" related stuff go into its own folder?

    - schema.rs

Should "schema" related stuff go in its own folder?

---

    sqlparser can't read IN table. SQLite's shorthand x IN texts (a table or CTE name after IN) is refused as unreadable SQL by the name rewrite

How do we fix that?

---

a: meh
b: definitely not
c: meh
d: definitely not

Are you unable to get help on this from the query planner that we just hooked into?

---

    this statement fails one step earlier, while its table names are being moved into the service's namespace.

Ok... and we decided to do that rather than keeping DBs unique per service in order to preserve atomicity/transactional integrity between services.

But: cross service transactional integrity was never gauranteed or relied upon in the Cloudflare runtime.  And it doesn't actually match up with the concept of services as "actor model".  We're doing a lot of SQL rewrite stuff that would be unnecessary if we were to give the services their own database instances.

Add a new topic: SQLite instance per service, not per space

And discuss with me.

---



