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
