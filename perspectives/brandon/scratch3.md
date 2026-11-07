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


