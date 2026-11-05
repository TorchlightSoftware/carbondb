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
