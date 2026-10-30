What if we were to approach this from a perspective of providing a decentralized runtime for ../otto?

So a schema like ./otto/services/books/common/schemas/account.ts:

    import {defineSchema, text, uniqueIndex} from '@tlsft/candle-db'
    import {ACCOUNT_TYPES} from './account-type.js'

    export default defineSchema({
      name: 'account',
      crud: false,
      generateClient: false,
      shared: {
        id: text('id').primaryKey().$defaultFn(() => crypto.randomUUID()),
        year: text('year').notNull(),
        name: text('name').notNull(),
        type: text('type', {enum: ACCOUNT_TYPES}).notNull(),
        // Subdivides a section; the statement gives the group its own subtotal.
        group: text('group'),
        // Presentation name where the ledger name reads poorly ("Assets" →
        // "Film Studio Equip.").
        label: text('label')
      },
      indexes: t => [
        uniqueIndex('account_year_name').on(t.tenantId, t.year, t.name)
      ]
    })

And routes like ./otto/services/books/src/routes/account.ts:

    import {z} from 'zod'
    import {CandleService} from '@tlsft/candle-service'
    import {notFound} from '@tlsft/candle-env/error'
    import {ACCOUNT_TYPES} from '../../common/schemas/account-type.js'
    import {getYear} from '../queries/year.js'
    import {listAccounts, upsertAccounts, deleteAccount, accountsInUse} from '../queries/account.js'

    const yearName = z.string().trim().min(1, 'Year is required')
    const source = z.string().trim().min(1, 'Account is required')

    const accountOutput = z.object({
      id: z.string(),
      year: z.string(),
      name: z.string(),
      type: z.enum(ACCOUNT_TYPES),
      group: z.string().nullable(),
      label: z.string().nullable()
    })

    export default [
      CandleService.defineRoute({
        // The year's typed Accounts, plus every name the year's ledger or opening
        // balances touch.
        path: '/api/years/:year/accounts',
        method: 'GET',
        input: z.object({path: z.object({year: yearName})}),
        output: z.object({
          accounts: z.array(accountOutput),
          inUse: z.array(z.string())
        }),
        handler: async ({input, query}) => {
          const year = await getYear(query, input.path.year)
          if (!year) return notFound(`No accounting year ${input.path.year}`)

          const [accounts, inUse] = await Promise.all([
            listAccounts(query, year.year),
            accountsInUse(query, year.year)
          ])
          return {accounts, inUse}
        }
      }),

      CandleService.defineRoute({
        // Type (or retype) an Account. One row per name: stating it twice is an
        // edit, not a second row.
        path: '/api/years/:year/accounts/:name',
        method: 'PUT',
        input: z.object({
          path: z.object({year: yearName, name: source}),
          params: z.object({
            type: z.enum(ACCOUNT_TYPES),
            group: z.string().trim().min(1).nullish(),
            label: z.string().trim().min(1).nullish()
          })
        }),
        output: accountOutput,
        handler: async ({input, query}) => {
          const year = await getYear(query, input.path.year)
          if (!year) return notFound(`No accounting year ${input.path.year}`)

          const [typed] = await upsertAccounts(query, [{
            year: year.year,
            name: input.path.name,
            type: input.params.type,
            group: input.params.group ?? null,
            label: input.params.label ?? null
          }])
          return typed
        }
      }),

      CandleService.defineRoute({
        // Remove an Account's row: the name goes back to the unclassified list,
        // never silently off the statements.
        path: '/api/years/:year/accounts/:name',
        method: 'DELETE',
        input: z.object({
          path: z.object({year: yearName, name: source})
        }),
        output: z.object({deleted: z.boolean()}),
        handler: async ({input, query}) => {
          const year = await getYear(query, input.path.year)
          if (!year) return notFound(`No accounting year ${input.path.year}`)

          const deleted = await deleteAccount(query, year.year, input.path.name)
          if (!deleted) return notFound(`No account ${input.path.name}`)
          return {deleted}
        }
      })
    ]

Would need to be executable within CarbonDB.  The services already have a JSON descriptor format: see ../otto/services/books/service-map.json

The schema, perhaps we could define it as zodly (https://github.com/TorchlightSoftware/zodly) and then translate it to Drizzle for the Cloudflare runtime, but we translate it to our own format for CarbonDB?  Can SQLite run inside the memory context presented by p2panda?  I kind of doubt that.  Maybe we could create similar semantics, but... hmm.  We interact with the data directly using SQL queries such as ../otto/services/books/src/queries/account.ts:


    import {and, asc, eq, sql} from '@tlsft/candle-db'
    import type {QueryRunner} from '@tlsft/candle-db'
    import AccountSchema from '../../common/schemas/account.js'
    import OpeningBalanceSchema from '../../common/schemas/opening-balance.js'
    import TransactionSchema from '../../common/schemas/transaction.js'
    import {writeChunked} from './chunked.js'

    const accounts = AccountSchema.table
    const transactions = TransactionSchema.table
    const balances = OpeningBalanceSchema.table

    export type Account = typeof accounts.$inferSelect
    // The query runner injects tenant_id on INSERT; callers never state it.
    export type AccountInsert = Omit<typeof accounts.$inferInsert, 'tenantId'>

    export function listAccounts(query: QueryRunner, year: string): Promise<Account[]> {
      return query<Account>(db => db.select().from(accounts)
        .where(eq(accounts.year, year))
        .orderBy(asc(accounts.name)))
    }

So this is probably more tightly coupled to Drizzle/SQL than maybe we would like for a p2panda port.

p2panda 0.3.0 release talks about SQLite, but this is in terms of an underlying persistence layer for p2panda itself, not as something we could use to hydrate data received over the wire...

I guess what I'm asking here is whether there's any hope of porting:

1) schemas
2) queries
3) route definitions

The route definitions would stay in javascript, but the Rust core would delegate to them and make the storage layer available.  And the rust core would need to expose SQLite-like semantics in order for the schemas and queries to be compatible.

What do you think?

---

    SQLite can run beside p2panda; there's no memory context to fit inside

    You're right about p2panda-store: it's storage traits plus SQLite implementations for p2panda's own logs, operations, groups, address book and orderer. Turning received operations into app tables is our job.

Ok.  So you're saying that we use p2panda to receive update streams to the data, but ultimately we could build our own "snapshot of current state" for each schema as a SQLite table, and then apply the log operations to arrive at that state.  Then at that point normal queries should apply.  Is that correct?

    we don't need to imitate SQLite semantics in Rust. We can run real SQLite and give the JS side a single "execute(sql, params, mode)" host function

I think we're saying the same thing, yes?

    Queries port unchanged because every query goes through QueryRunner

Ok.

    Tenant scoping carries over. The runner's tenant injection maps naturally onto a p2panda space.

Uh... I would say that different tenants almost certainly should be sharded, and should have P2P spaces that don't overlap or share nodes with each other.

    Handlers would run in QuickJS
    - input, query and zod work as they are.

Nice.

    - identity becomes the caller's device or identity key.

It's the user/person.  Which can have multiple devices.

    - connect becomes a service-key call over iroh. service-map.json already keys routes as GET [books]/years/:year/accounts, the same grammar as components/service-calls.md.
    - files (line 35) becomes p2panda-blobs.
    - request: Request (line 28) and env need small shims.

This seems doable.

    zodly isn't needed for the port, and it doesn't cover SQL yet

I know that Claude.  I created it with your help.  I'm very familiar with what it can do.  I was suggesting it because it could potentially be a format where the schema is defined in JSON, which would be more readble by rust code than a schema defined in typescript.  But yes, maybe the Rust code doesn't need to know or care about the internal data format of our app.  That would be really ideal in a way - it separates the concerns really nicely and we don't have to teach Rust about our data models or duplicate any logic there.

    The real question: who writes, and how do concurrent writes merge?

    1. Single writer per service

It's a possible configuration, but it doesn't really exercise the P2P model other than peer discovery and connectivity.  Still it will be appropriate in some cases.

    2. Row-level CRDT SQLite.

This gets more at the heart of "why we're doing this".  I'm not sure why you're introducing sqlite-sync or cr-sqlite?  Do we honestly need that tech?  Is that basically implementing our method for applying the ops?  Are both layers independently necessary and non-redundant, sqlite-sync (say) and p2panda?  I guess the one is sync, and the other is network?

    3. Replicate route calls themselves. Each PUT or DELETE becomes a p2panda operation

I don't know if REST or gRPC calls translate well to CRDT land.  They don't fold well into an op log.  I forget what the mathematical terms are... something about partially-ordered lattices...  It's an interesting idea, I would just want to make sure the semantics of it are well defined and follow the rule of least surprise.

We also already have an audit log implementation in:

    ../otto/core/candle-db/src/schemas/audit.ts
    ../otto/core/candle-db/src/queries/audit.ts

Perhaps this is "good enough" semantics for business logic, while giving us a built in access log (who dun it?) that is pretty readable and easy to build a human interface around.

    account will conflict under offline merge as written
    gives each row a random id

Yeah, adaptations for the port will have to be made.  That's reasonable, not something I'm concerned about.

---

    In a handler, query(db => db.insert(...)) can't just change the local table. It has to produce an op that goes into the tenant's log, and then every replica, this one included, applies that op through the same fold.

Yes.  So all mutations go through the op log, and nothing mutates the table directly.  So we need two transforms:

    - SQL mutation Query -> Ops
    - Ops -> applied to table

Are you thinking the Javascript code should be responsible for that?  It makes sense in terms of the current separation: keeping rust out of the schema.  But it also makes some of the Rust qualities less valuable: Rust is faster and safer.  Still though, it might give us an initial implementation that gets us off to the races, and then we refactor some components into Rust as a performance and security improvement.

    We don't need sqlite-sync

Concur.

    A join-semilattice is a set of states with a merge that is commutative, associative and idempotent

Yeah, what you said.

    Your audit log is the right shape, but replicas would disagree as written

I'm fine dropping the audit log or letting it evolve to something more useful for our needs.  Either way.

    Do you mean that relays and always-on peers are dedicated to one tenant

No, I mean that there's a logical sharding of tenants regardless of what devices they happen to be on.

    one SQLite file per tenant, several open on one device

That sounds more appropriate.  But I don't know.  It may be something we don't need to finick with in the initial port, and we can come back and look at whether we want to optimize in some way.

    the person comes from looking that device up in the person's device constellation, and the audit log should record both

Yes.

    Refactors to audit.ts

I'm surprised you're planning to use it given our other discussion.  I think we should come back to the three write models you mentioned: We have not decided what to do there, we've merely uncovered and discussed context.

Please create a new agenda with the items we are actively discussing, with the goal of porting an Otto service.

---

    The biggest gap: each peer has to check other peers' ops when it applies them

Ah.  This moves me towards "we need to turn service calls into the op log".  It's not sufficient to simply implement sync at the SQLite level: of course SQLite doesn't enforce business rules.

This also points to why Holochain is designed the way it is.  You have something that looks very similar to services there... they just called them something weird.  DNA or something.  Everything in Holochain is a metaphor you have to go research to find out what it means.

Are there necessary invarients in how services need to work in order to be suitable for this kind of use case?  Is that something we can shoehorn our existing services/framework into?

I think the goal for this project should be: Export a suitable abstraction for building an Otto runtime on top of.  Include a [chat] service that loosely resembles the shape of an Otto service, while carrying very minimal dependencies.  [chat] should be tested and runnable on CarbonDB.  I'm expecting CarbonDB to be a single binary file (could it be WASM/WASI?)  With nice javascript bindings for me to build on.

Are we missing semantics that need to be present in the decentralized model?  Do our apps need to be describing rules for group membership, pull, read, write semantics... do these need to be described more directly in a way that we can't simply "assume" or autogenerate from the Otto definitions?

    Building host-node first helps all three hosts

Noted.

    Otto's custody plan chose single writer; multi-writer is a deliberate departure
    - master-plan.md:64 says "Single writer per workspace, by design."

You're taking that way out of context.  That's a runtime where we download an encrypted blob to the browser and run services locally on it.  You can't apply a statement about a single (unimplemented) runtime to the entire rest of the framework.

Can you put your list of things to verify under perspectives/brandon/verify.md?

Can we talk about: 1) lessons learned from Holochain, 2) whether we are now implementing something similar on top of p2panda, 3) whether p2panda itself has any suggestions for what this layer should look like, or whether there are examples in the wild built on top of p2panda.

Please set aside the other things we are discussing as agenda items.

---

    We'd add three things: op types, validators, and a projection of ops into SQLite

ops: Service Key + Fields + ... is there some secret sauce we can sprinkle on it to make it a join-semilattice?

validators: We already have input/output schemas defined as zod/zodly/zodex.  Is that sufficient and equivalent?  We don't need those to be defined as functions, particularly zodly is a viable format which is just data.

    p2panda deliberately leaves this layer to apps

Yes.  Praise Zod.

    - Their current guidance (2025-07-09 post (https://p2panda.org/2025/07/09/streams-transactions-crash-resilience.html)):
      - every peer builds its own state from the ops;
      - each incoming op is one atomic transaction shared with the app's state;
      - the app acknowledges every op;
      - "Applications should always have the last say as to whether or not they want to reject an operation."

Ok.  So I think we are thinking about this correctly: it's our job to break this down into "what are the atomic operation chunks" and "what is the valid format for them" and "how do they get applied to application state".  And I think the service definition as it stands is a pretty good model for that.  And it sounds like our usage of SQLite is very compatible - p2panda updates their "system" records in SQLite, and we update our "application" records likewise.

    Dash Chat was built on Holochain and is now being rebuilt on p2panda.

Ouch to Holochain.

    - Ops are domain facts: Message, Edit, Delete, Reply.

Doesn't that line up completely with our service calls?

    My proposed invariants:
    1. A write is a named, versioned op: a domain fact, not SQL.

Service Key + Fields + ...

    2. Each op type has a validator that depends only on the op, the author's role at the time, and ops it references by hash.

Zodly (approximately, don't nitpick me).

And the last part - ops referencing each other by hash - indicates a structure in the op log that is not present now.  That's interesting, and gives us some clues as to how the translation of SQL mutations needs to occur.

    3. State in SQLite is a pure fold of valid ops, and can be thrown away and rebuilt.
    4. The author picks ids and timestamps, and they travel in the op.
    5. Rules that need global knowledge, like uniqueness or a non-negative balance, can't be guaranteed. They have to be modelled as visible conflicts or resolved deterministically.

Ok.  No major arguments there.

Yes, apps have to declare membership and access rules; Otto definitions can't generate them

    Things only the service author can state:
    - what one space contains (the tenant, a year, a chat room);

I'm not clear on what a "space" is.

    - which role may author which op type;

I think this is already answered by our current access control, which is based on service keys.

    - who may add and remove members;

Can this be reduced to a service call itself, and therefore be access controlled in the same way?

    - the conflict policy for each op type;

Possibly extra meta data on the service call definition.

    - retention and pruning;

Of ops?

    - the encryption scheme: whether late joiners can read history, or messages get forward secrecy.

Hmm.  Services are able to modify multiple underlying models.  Is that ok, in terms of how we're thinking about Ops?  Or could it be derived?  Could it be said that `POST [books]/year/2025/close` interacts with the `transactions`, `reports`, and `payroll` tables, and therefore is comprised of these ops?

    POST [books]/year/2025/close {...fields} -> transactions
    POST [books]/year/2025/close {...fields} -> reports
    POST [books]/year/2025/close {...fields} -> payroll

Is that necessary in order for the underlying models to work correctly?

    Read gating, though, reduces to space membership

Eh... again, help me understand better what spaces are and how to map them to Otto architecture.

    A single binary is plausible; WASI probably isn't

Good to know.

    a JS app (Node or Vue) driving the binary as a library

This.

---

    Convergence then needs one rule: the SQLite projection must depend only on which ops exist, never on the order they arrived in.

Yeah, but you can't just declare that arbitrarilly over a code base that never enforced that before, and doesn't have any means for testing whether it is true.

    The ingredient to add to each op is a list of the ops its author had already seen, by hash

That's something like the audit log... the audit log would need to be brought into better alignment with p2panda's needs.  And then mutation queries could be analyzed and based on what they are reading, reference the source Op.  Sounds possible but involved.

    Proposed op shape: service key + fields + op hash + author + timestamp + seen ops + schema version.

`device` is more appropriate for the sync protocol.  `author` still needs to be present for access control and audit though.

    today roles and features arrive as claims in a JWT issued by [identity]
    With no central issuer, role assignments have to become ops in the log too

Eh... someone is always the owner.  If we're joining Bob's Clubhouse, then Bob should have something equivalent to the [identity] service that will perform this role.  If anything things get easier because we can dispense with email as a primary means of verification of identity.

    Relations: rules that reference other ops, such as "only the original author may edit", "the target exists", or "the year isn't closed". Zod can't express these

...I'm not sure we should be trying to jam cross record validation into our field validation layer?  Can we not back up the train if we try an Op and it gets a 500, or a 404, or any other error code?  Does the validation layer need to 100% guard consistency of the data?  And if it does... why does the business logic exist at all?  It feels like we could easily get to a point where the validations significantly overlap the logic of the handler itself, and that's one of the areas where I think Holochain went too far.  Ok, maybe if you have your mutations defined in a declarative language for which you can transform it into both aspects.  But I'm not a fan of duplication.

    - Call only: every peer re-runs the handler to apply the op.

Ah, I think I answered my own question.  The Otto service model is essentially actor model: all of the SQLite tables accessed by a service can only be accessed by that service.  They are encapsulated.  Therefore it doesn't matter that they happen to be stored in different tables.  In terms of the state synchronization model, the service has ownership and therefore we don't need to dive into specifying tables.

My only question is what happens when events are received out of order: let's say Martha was out in the woods and accidentally deleted payroll, and then comes in Monday morning and syncs.  Meanwhile other people had submitted and finalized their pay periods over the weekend.

Maybe a contrived example.  But point is: I don't know that calls to a typescript function that performs mutations can be reversed, or re-ordered...

Maybe you have:
    POST [books]/year/2025/close (join-semilattice)
    -> SQL on transactions (as join-semilattice)
    -> SQL on reports (as join-semilattice)
    -> SQL on payroll (as join-semilattice)

E.g. you have in essence two different layers: the service and the underlying storage, and the ops on each need to be structured as a join-semilattice so that they can be properly reordered.

Please tell me I'm overcomplicating it if that's true.  I don't want to add any unnecessary layers.

    Question: how far do we trust space members not to forge ops?

I'm basing my decision making more on "what is an implementation that gives eventual consistency".  I'm not really war gaming this yet.

    Mapped to Otto:
    - Space is a tenant, or a slice of a tenant whose members all share read access.

I think a space is probably more analogous to a "role" in the Otto system.  Which yes, is also tenant scoped.

---

    The peer rewinds to a saved copy of the SQLite state from before that op, then replays forward

That sounds expensive to implement, expensive to store on disk, and expensive cost to runtime latency.

    Handlers only have to be deterministic. Given the same input and the same prior state, they write the same thing

Assuming we have a complete view of "input" and "state"... Can you gaurantee that in a Javascript function?

    For books, at hundreds of ops a year, replay is cheap

That's a very lazy stress test Claude.  I would like us to have higher standards.

    In replay, a handler that returns 404, 409 or 500 rejects that op. It does so the same way on every peer

Getting into a state where all the ops bork in a long chain sounds like a nightmare.  I mean I think we need at the very least a rule that if you were offline for a certain amount of time, GFYS, we're not taking your data and you can start over.

    What's left is design work:
    - Show rejected ops to the people who made them and to Martha.
    - Domain choices such as soft deletes, or requiring a manager role to delete payroll.

Well that's valid.  I had thought about a standardized interface for viewing Op history, something that would let you troubleshoot and find the culprit, and revert it.  The benefit with this type of system is you never lose data.

    A hybrid logical clock is simpler than tracking "seen" ops

Yes.  That's better.  Then we don't need to describe relationship to all other ops, we just need relationship to time.

    the embedded JS engine supplies crypto.randomUUID, Date.now and Math.random from values derived from the op, and refuses connect and fetch during replay

I think you're proposing rules that are contradictory to normal service programming, and the depth to which you've gone down this track of wishful thinking is... less than impressive.

    If a service's ops were split across role-spaces, someone in only one of those spaces couldn't rebuild that service's state.

Well yes.  But if you give a person complete knowledge to rebuild the complete state of a service, then there is no meaningful sense of privacy or boundaries within the company.  You're proposing a "solution" to meet the demands of CRDTs that will piss off every CEO in the world.

In one sense... maybe it's just fine and accurate:

1) a traditional business's services NEED to be deployed in a client/server architecture; Martha the secretary can send messages to the business services, but she doesn't ever get the complete state
2) a chat has a clear membership... can be segmented thusly, carries a different model; this is a "document" level space
3) trello board among friends... sure, we all see the shared board state; this is a "document" level space

So again, I think we need to make case by case decisions, and it won't be appropriate to deploy things like [books] as an app in shared custody between actors that shouldn't have read access to the data.

Can you update the agenda with what we've discussed so far?  I feel like some topics should probably be removed or updated based on how the conversation has evolved.

When done with that, give me the "next" agenda item.  Let's cut through the noise and try to get an implementation started.  What do we need to decide on to start moving?

---

    Where service handlers run: in the Node process

Yes.

    The Node binding: a napi-rs native module

Preferred.

    Next: Choose how replicas apply call ops in a document space

    An op is a service call: service key and fields, plus op hash, device, author, hybrid-logical-clock timestamp and schema version.

As I told you before: `device` is more appropriate for the sync protocol.  `author` still needs to be present for access control and audit though.

Please add `device` to your list.

    The service owns its tables like an actor, so ops aren't described table by table

Probably.  But it's a question of whether we can establish a consistent merge strategy at the service call level.

    Rewind-and-replay is too expensive

Probably.  But could be performance tested.

    Dash Chat avoids rewinds by making each op's effect order-independent: records are keyed by op hash, only the author can edit, and deletes are tombstones.

Yes, but:

1) a chat already looks like an append only log, so in many cases it's low hanging fruit and doesn't stress a decentralized model in the same way that other business logic will.
2) custom wiring the models with CRDT semantics might be practical at a small scale, but I don't think it's very practical for larger domains.

I'd like to have a system where it's more like "models described in the traditional fashion will work with LWW defaults", and "you can add schema meta data to optimize the model treatment".  This allows for more progressive adoption.

    Proposal 1: the op carries the call plus its row writes (my recommendation)
    Business rules are checked only on the author's device

That's unacceptable.  No business domain would find that acceptable.

    Proposal 2: receivers re-run the handler as each op arrives
    Handlers must be written to be order-independent: insert-if-absent by an author-chosen id, updates guarded by the clock.

I think we need to look into creating tooling to help with this: can candle-db support a mode that enforces the varients?  Is some transform of the mutations necessary before application?

---

    This is the CALM theorem:
    - Rules that only add facts converge with no coordination.
    - Rules that can flip need an agreed order.

Analyze MicroQL syntax with this in mind:  ../microql

Keep in mind that underlying this are service calls, which have all the properties we've been discussing.  But I'm wondering if MicroQL itself has any potential for bearing the kind of properties that are being talked about in Bloom:  http://bloom-lang.net/calm/

    Yes, candle-db can enforce this, by transforming queries at the QueryRunner
    QueryRunner already rewrites every query to inject tenant scoping

Yes, I was thinking along those lines.

    1. One order: every replica sorts ops by (clock time, device, op hash). Re-running handlers in that order gives every replica the same accept or reject, with business rules intact.

    2. SQLite's session extension can produce the inverse of a set of changes.  To rewind, a replica inverts only the ops after the late op, not the whole history.

Ok, that sounds attractive!

    3. Rewind only on overlap: A late op whose reads and writes don't overlap later ops applies in place with no rewind.

Yes, that sounds like movement in the right direction.

    4. Last-writer-wins comes from the order itself.

Yes.  And better mergeable data structures have to come from the schema design.

    5. Ids and timestamps come from the op.

Ok.

    Handlers must not call Date.now, randomUUID or Math.random directly; they'd take time, ids and randomness from the op context instead. There are 44 direct calls in Otto service code today.

Very acceptable.

Ok, can you get rid of the "Dropped" items in the agenda, and update for this scope and target?

---

    a write such as PUT or DELETE is only monotonic if the service says so

Should be considered as such because that's what the HTTP verb is promising.

    Each service key would declare its class

Aren't you reinventing HTTP methods?

    It hashes every value a call reads, and marks a phase stale when an upstream result hash changes

I think you're mistaking a real runtime optimization for a persisted artifact that doesn't exist.  MicroQL doesn't have much of a concept for what happens on future runs.  It doesn't store state, except as a performance optimization.  The services are responsible for persistence.

I think what's probably a truer statement to be made: If we can make certain gaurantees about services, then MicroQL could potentially make use of that.

    Classify service keys as monotonic or order-sensitive

Is that deriveable from the HTTP method?

---

    CarbonDB knows ops, order and SQLite; nothing about schemas or routes

Ok, so CarbonDB is essentially exposing:

- I'm going to carry the abstraction of Ops as Service Calls
- You get to define what your Service Calls look like, and tag them with meta data
- I'll take care of rewinding if necessary, and merging state
- You can listen for state changes (probably?)

    Everything we've agreed that doesn't depend on schemas is CarbonDB's:
    - From p2panda: the node, spaces and membership.
    - CarbonDB's own goals from the README: device constellation (QR enrollment, revocation) and the blind relay.
    - The op envelope: device, author, hybrid-logical-clock time, schema version, with the body opaque to CarbonDB.
    - The shared order: sorting by (time, device, op hash), and removing duplicates.
    - SQLite: an execute(sql, params) call for the app's tables in each space.
    - Undo: each op's changes recorded as a SQLite session changeset, inverted to rewind.
    - The apply-and-rewind loop. CarbonDB owns the order and the changesets, so it decides when a handler runs or re-runs. JS supplies the handler as a callback.

Great clarifications.  Write these down in perspectives/brandon/design-goals.md.  Give a sketch of the overall API footprint.

The rewind can happen via SQLite "session" extension, right?  Make sure that gets recorded as a target: certainly I will prefer to use it as long as we determine it to be compatible.

    My recommendation is that CarbonDB drives the apply loop: for each op, it calls the JS handler

What's interesting about this is it becomes essentially the transport layer.  So the handlers are no longer called by HTTP, but by the replication protocol.  I like it.

    That handler calls execute back into CarbonDB synchronously while the apply transaction is open

I don't know whether that's necessary or desirable.  What is "execute"?  When CarbonDB calls the JS handler, how does it deal with async calls (which most of them will be)?  Is that what you're talking about?  We need some kind of bridge: JS Promise -> call Rust callback when it's done?

    Read tracking can start in CarbonDB. SQLite's authorizer callback reports every table and column a statement reads. That gives schema-agnostic, table-level read sets in Rust. It's coarse, so it would rewind more often than needed. QueryRunner can narrow it to rows later.

I'm not sure what you're talking about.  Elaborate on the difference between this and the "session" extension approach, and the QueryRunner optimizations you're talking about.

    Demo scope. My proposal: the [chat] demo runs on a minimal JS runtime in the carbondb repo with plain SQL, matching "minimal dependencies".

Yes.  I think that's great, if I understand it correctly.  You're saying that we care about Ops and SQLite writes, and we don't care what happens in between.  So we can create a demo without any abstraction layers or build process that still fulfills the fundamental contract for "what a service is".

Ok, so what I'd like to see is:

1) update the agenda
2) start sketching out examples of what the calling API will look like
3) maybe start with node.js [chat] service minimal declaration
4) get it to run on normal node.js single node runtime with SQLite
5) then create a new mode where we add carbonDB bindings.  We don't need these to compile or run properly yet, but let's agree on the shape of them, that the call ergonomics look right.
6) then proceed with the further buildout of CarbonDB

---

    CarbonDB owns each space's SQLite database, because the session and the read tracking have to wrap the same connection that does the writes.

Interesting.  Aptly named then: CarbonDB.

    Database calls inside the handler are synchronous and run inside the open transaction.

I will say again: this doesn't make sense to me.  It sounds like you're saying that all database calls would essentially queue up and be committed as one transaction.  I think that would blow up our existing logic that expects responses back from each query, and which makes decisions based on those responses.  Am I misunderstanding what you're saying?

    When the Promise resolves, CarbonDB commits. When it rejects or returns an error, CarbonDB rolls back and records the op as rejected

This sounds brilliant: transforming a service call into a database transaction.  I just want to make sure my above concerns are addressed; that we're not blowing up the current expectations of the imperitive code.

session and authorizer: also sound really brilliant in empowering our use case.  I'm glad you surfaced them.

    Question: does the API shape in the doc look right: serve, call, apply(op, db), subscribe

Show me examples.

---

    openNode
    createSpace()
    invite()

Hmm... are these standard from the p2panda nomenclature?  They're not terrible, but they don't really describe things from within the service domain.  I would rather see that CarbonDB gives me abstractions that let me "initialize my identity on a new device", "add an additional device to my identity", "register these services", "call this service in a personal context", "call this service in a shared context".

So what I'm wanting to see are the interactions between [chat] and CarbonDB, and some of what you're showing me look more like things that should be internal implementation code of CarbonDB.

Maybe we should do more of a story workshop to get clear on this?

---

    A context plays the part a tenant plays in Otto

1) let's just use "space" so we don't need to arbitrarilly change the term.
2) I keep warning you not to make that equivocation... there will be multiple spaces within a tenant.  A many to many relationship between roles and spaces.
3) maybe... space = [tenant, service]?  I've rejected that too, but it might be worth coming back to because services and service calls are so fundamental to the model and are a natural boundary.
4) I think perhaps p2panda's notion of permissions has more to do with "where shall it be hosted" and perhaps our access controls are an independent concept.  IE maybe the answer for "Martha the secretary doesn't get access to everything" is "don't host on Martha's devices, let her access over the network".

    Draft stories to start from

    Three actors: a person, a service author (whoever writes [chat]) and the app (Node or Vue).

    1. A person initializes their identity on a new device.
    2. A person adds another device to their identity by scanning a QR code.
    3. A person removes a lost device.
    4. The app registers its services ([chat]) with CarbonDB.
    5. A person calls a service in their personal context.
    6. A person starts a shared context, such as a chat room.
    7. A person brings someone else into a shared context.
    8. A person calls a service in a shared context.
    9. The app listens for changes in a context in order to render them.
    10. A person learns that one of their calls was rejected after a merge.

Add it to design-goals.md

    We agreed membership changes are service calls. That suggests POST [chat]/rooms and POST [chat]/rooms/:room/members

Mmm...  I think it's probably more true that [identity] needs a different implementation for the p2p runtime.  Or at least it needs additional endpoints to support this use case.

I don't know if you can make individual services responsible for this... it feels more at a meta service level:

1) I create a space, naming the service(s) that belong in it
2) Add members
3) Now we collaborate

And separately:

4) Give Martha remote access, specific roles, but she's not a "member"

Like these all seem like they're [identity] operations...  they set up the identities, services, and assign the permissions for making calls.  Or maybe the service provisioning portion could go in a [provision] service, separate from [identity].  But [identity] will still need some updates in order to understand this new dynamic deployment of services.

Go ahead, walk me through one story at a time and sketch out what the API will look like for the Node.js caller.  We'll discuss and agree upon each one before proceeding to the next.  Can you create a new agenda for these stories, so we can have clean context?  And we'll pop back to the previous agenda once we've got a clear spec and stories defined.

---

    My sketch

This is great.  Love the format of talking story by story about the API footprint.  Great way for us to get clear.  Shall we store results in `stories/` once agreed upon?

    import {createIdentity, openDevice} from '@tlsft/carbondb'

1) createIdentity and openDevice are mismatched.  It should be createIdentity and... resumeIdentity?
2) devices and identities are independent:

    ID1 --has--> [D1, D2, D3]

Identities are logical, and devices are physical.  They probably need independent records.  The device itself probably can be bootstrapped without much interaction other than giving it a pet name.  Should a device do any kind of fingerprinting of its surrounding environment?  Should it contain certain metadata like hardware, operating system, etc?

We should record schemas for the different CarbonDB system records.  Shall we use SQL, and make executable schemas which actually are then brought into use by the Rust code?  Place in `schemas/`?

    // First run, first device: a new person comes into being.
    const {device, paperKey} = await createIdentity({dir: './carbon', deviceName: 'laptop'})
    // device:   {person: 'p:7Hq…', id: 'd:K2x…', name: 'laptop'}   (plain data)
    // paperKey: 'twelve words …'   returned once and never stored; the app shows it for writing down

    // Every later run on this device:
    const device = await openDevice({dir: './carbon'})

Most uses of "device" should instead be "identity" which is the more salient entity to which permissions and custody are attached.  However:

1) device SHALL have its own generated assymetric keys
2) said keys SHALL NOT leave the device or be communicated to the outside
3) device only signs, authorizes sub keys
4) device to device encrypted communication is the baseline security layer
5) identity cryptographic keys SHALL BE transfered

We need to work out further invarients for what safe, secure communication looks like while retaining good ergonomics.  KeyBase worked out most of this and we can follow their example.

    Everything else takes device as its first argument, for example call(device, space, key, input), rather than calling methods on an object.

Need to discuss what methods need [device], [identity], or [device, identity].  Perhaps we have a "context" which contains current [device, identity].

    Open questions
    1. Is a person's display name part of the identity, or profile data held by [identity]?

Profile data sounds fine for now.

    2. Does creating an identity also create the person's personal space, so story 5 needs no setup step?

In terms of an eventual UI: we're going to have a number of steps that occur pretty atomically from the user's perspective:

1) ask the user for their name (pet name for self)
2) declare a new "device"
3) declare a new "identity"
4) declare a personal "space" and add [identity], maybe [contacts], [chat], [wallet]

But in terms of CarbonDB, I think it's still appropriate for these to be independent operations, called one at a time by the app.

    3. For the demo, can keys live in files under dir, with the operating system's keychain and the ask-before-signing boundary coming later?

You can put them in SQLite in plaintext.  This should be only available in a dev/test mode.  The library's production mode should require other forms of secure storage for secrets.  TBD, we don't need to design that now.

---

    In Keybase there is no single identity private key:
    - The account is a sigchain: a signed log of device additions and revocations, each entry signed by an existing device.
    - Each device holds its own signing and encryption keys and never shares them. That matches your rules 1–4.
    - A per-user key is encrypted to each device and replaced when a device is revoked. That's your rule 5. Replacing it means a lost device can't read anything written afterwards.
    - A paper key is simply an offline device, so recovering means enrolling a new device with it.

Brilliant.  Make it so.

    Device metadata: self-declared, no fingerprinting

Great.

Ok.  In terms of your code:

1) I like resumeIdentity rather than resume
2) Do you need to have any "device" interactions?  Can that just be implied?  e.g. createIdentity, resumeIdentity, joinIdentity...
3) I don't like providing the directory each time.  I asked you not to store things in files but to put them in SQLite.
4) Initialization for CarbonDB should occur once in a toml config file (where is SQLite, what settings is it started under).
5) Devices and Identities need to receive a slug.
6) Need a list() API (separate story) in order to support profile selection.

---

    // First run: the device is created implicitly from [device] if this database has none.

No, creating an identity always creates a device, because devices are identity scoped, as you yourself just declared in the KeyBase invarients above.  And the presence of device in the toml is also suspect: you could store defaults in there I suppose, but that's not something I asked for... I'd say just get rid of that.

    Does each identity on a device get its own device keys?

Yes.  Again, you already answered that with KeyBase's invarient.  Please record things we're in agreement about and quit asking me.

    Identity slugs are unique on this device, and device slugs within an identity

That's fine.  Again they're based on pet names.  They're local only.

    The id (from the public key) is the real name

Yes.  Probably a canonical hash of the public key.

    My proposal: an explicit configure(path) at startup

That's good with me.

I'm good with the API, let's move to the next.

---

    Which device shows the code
    Keybase supports both directions

I would like to support both directions.  That leads me towards something like:


    // on a new phone
    const join = await joinIdentity({device: {name: 'phone', ...meta}})

    // pops up QR code to be scanned
    display(join.qr)

    // scan a QR code for the other device
    qr = scan()
    join.apply(qr)

Or initiated from the other side:

    // on an existing identity member device
    identity = resumeIdentity('brandon')
    join = identity.addDevice()

    // the rest looks similar
    display(join.qr)

    qr = scan()
    join.apply(qr)

---

    One scan is enough; the app offers both and whichever happens first wins

I know that.  I presented demonstration code for both.  Exactly what happens would be up to user interface.

    Methods on handles

I prefer this for the Javascript API.  It's more idiomatic for JS.  If you want to make the Rust calls as "functions taking handles" that's fine - just abstract over it in the translation layer.

Ok, let's move on to the next item.

---

Proposed safeguards

1. Single-use: yes
2. Short-lived: yes
3. Announce every join: yes, CarbonDB should, but it's up to the interface how to (and whether to) display this
4. Optional: compare a short code

Generalize this: CarbonDB offers a slot for "vouchers".  The application may provide its vouchers, and and the application on the other side is responsible for interpreting them.  This lets us build:

- I can see cryptographically verified connections to twitter, facebook, google, email, etc.
- I can see a profile picture

If that sounds good, let's move back to story 14.

---

    1. Are services registered on the carbon handle, app-wide?

Services are registered within a `space` which includes 1 to many `identities`.  So you need something more like:

    // I am implicitly a member, as the creator
    rph = id.createSpace("Randall's Playhouse")
    rph.register(['identity', 'chat'])
    rph.invite("Sally")

They may be "code in this app" but the data within them, and the "space" that it represents needs to have a lifecycle, membership, and access controls that are defined by the user.  This is the difference between a class and its instances.

However, maybe you're asking a different question?  At some level a catalog should be defined, the catalog of supported services from which users can select.  That might be registered on `carbon` itself...  At some point we will have a whole lifecycle of downloading, installing, updating, removing services.  But that's way out of scope for the demo.  I would say though:

1) don't hard code, or place in the toml file the services to be loaded
2) instead rely upon dynamic registration by the app, and store the registrations in the SQLite database
3) registrations will include file paths as to where the javascript handlers are stored... That shouldn't be put in the database.

    2. Is a service's schema version the number of its migrations, carried in every op?

Interesting.  Ok.

    3. What happens when a space names a service this app hasn't registered?

Flip out.  I'm sorry Dave, I can't do that.

---

    // what this app can run; repeated on every start

You still didn't follow directions.  I told you that adding to the catalog only needs to happen once, and you should put it in the database.


---

    I'd go with an immutable set of service names, fixed when the space is created

Let's do that now.  It lets us build the interface for which we have defined behavior.  It leaves it open for us: we can later build an interface for adding to an existing space, if we have a more specific use case for that and an expectation for how it should work.

---

    const personal = await id.createSpace({slug: 'personal', services:

Your thoughts on accepting `slug` explicitly vs. accepting `name` and calculating `slug` internally?

    await personal.call('PUT [contacts]/contacts/sally', {name: 'Sally', key: 'i:Q9c…'})

Now we get to the meat: good stuff.

    Is a personal space just an ordinary space with one member?

Uh.  It starts with one member.  A person should probably be warned before sharing with another.  But lets say someone has a caretaker, or power of attorney.  There are cases for sharing even what you might think of as personal space.

    Do spaces get local pet-name slugs

Yes.

    Is a call that the caller's own copy refuses left unpublished?

Hmm.  Is that behavior left undefined as far as p2panda goes?  It's up to the implementor?  I'm fine with the behavior you've described: try local first and if it doesn't pass muster here it certainly shouldn't be broadcast.

---

    Local refusal is our rule to make

Ok, I'm a yes to this.

    Slug or name: I'd accept a name and derive the slug

Yes.

    My proposal: createSpace({name, services}) derives the slug from the name (randalls-playhouse, with -2 added if that slug is taken locally) and accepts an optional slug to override it

Yes. But: -N with iterating integer.

---

    My proposal: roles stay in [identity]. Who may call which service key is application policy and changes with the app

Yes...  But I'll repeat that p2panda access controls are something distinct.  This determines "where it's hosted" which is a different question than "who has access".  There should probably be a bridge between the two, in the sense that:

1) if I'm a member of a space, then implicitly I must have a record in [identity] within that space
2) [identity] may carry entries that are NOT members
3) members authenticate automatically through cryptographic proof
4) non-members may still utilize magic link in email, as currently implemented
5) non-members access the services remotely

---

And just to clarify further: that doesn't mean that CarbonDB needs to know about [identity].  CarbonDB can accomodate this by providing a listener for various state changes including `space` creation, membership changes, and dissolution.  The Otto specific runtime would just bind to that and update [identity] records.

    Does every space run [identity]?

In Otto, yes.  But CarbonDB should not presume Otto specifically, or constrain the app to behave exactly like Otto.

---

    My proposal: CarbonDB delivers lifecycle events through the apply loop, at their position in the log

Interesting.  I love it actually.  Let's say we have an implicit [carbon] service, and instead of providing certain API calls we just implement them as service calls within CarbonDB.  Anything that needs to be a part of the log should happen this way.  Perhaps nearly everything should happen this way.

    Unverified: p2panda's membership rules and our op order have to agree
    I'll add this to perspectives/brandon/verify.md

I think a lot of the stuff in that doc is out of date at this point.  Please clean it up and get rid of irrelevant stuff (prior art, anything else we've already explored and decided upon) before updating it.

    carbon.on('POST [carbon]/members', (op, db) => {
      db.run('INSERT INTO member (identity) VALUES (?)', [op.input.identity])   // Otto keeping [identity] current
    })

Also interesting.  You're treating the service calls as a message bus.  I'm not against this... but I think we need to distinguish between:

    carbon.handle('POST [carbon]/members') // carbon handles its own namespace, others are not allowed to
    carbon.listen('POST [carbon]/members') // others can listen; listeners do not affect the output of the call, and cannot cause it to fail

---

    A listener that writes tables still changes state, so it needs two guarantees

Can we disable/not offer any state changes from a listener?  It just shouldn't be provided at all.  The only thing it can do is call other services.  So then the state changes if any belong in those other services.

---

What if we do:

    carbon.listen('POST [carbon]/members', {
        in: ['POST [logger]/log'],
        out: ['POST [logger]/log']
        })

IE: listeners don't even have a handler definition.  They're pure JSON.  All they can do is forward the inputs and outputs somewhere.

---

    Plain forwarding leaves the translation to the receiver, so [identity] offers a route that accepts [carbon]'s input:

Precisely.  This interpretation needs to happen somewhere, and no one know's the caller's intent better than the caller themselves.

    If mapping is ever needed, MicroQL-style references ({id: '$in.identity'}) would extend this without leaving JSON.

Yes...

    Every device in the space must run the same listeners
    A listener registered with carbon.listen on one device's handle exists only on that device

Yes.

    carbon.call('POST [carbon]/listen', {
        to: 'POST [carbon]/members',
        in: ['POST [logger]/log'],
        out: ['POST [logger]/log']
        })

    listeners fire only when the triggering call succeeds

That's fine for now.  We don't have a specific enough use case to discuss further.

Are you ready to move on to the next topic?

---

    await rph.call('POST [carbon]/members', {identity: sally.id})

Interesting.  Again, each space is a message bus.  So you're essentially adding one more member to the message bus.  Space ID is implicit in the handle.  Looks good to me.

    How does Brandon learn Sally's identity id? My proposal: a contact exchanged by QR code or link, using the same mechanism as joining a device and able to carry vouchers

Yes.  QR code easiest in person.  People could also add themselves to professional directories, leave a link in their email signature, etc.  Let's say someone builds a general purpose relay server for relaying p2panda or carbondb... That could have a registry in it.  We don't need to build or invent any of that now.  QR is sufficient to assume.

    Does membership take effect only when Sally accepts on one of her devices?

Yes.

---

    The handler sees op.author (the identity) and op.device. Checking whether the author may make the call against [identity] is the Otto runtime's job

Yes.  Just to clarify, currently in Otto the identity is passed by JWT (in HTTP headers I believe, not in call params).  The exact format of author and device can shift from what it is now in Otto... but should be roughly compatible to serve the same function.  I recommend JWT.  If we're not using JWT, I would say "why not?"

    Does a call resolve as soon as this device has applied it?

You already asked this, and I already answered yes.  It bugs me when you ask the same question multiple times.

    Are reads always answered from this device's copy, even if it's behind?

Yes.  That's inherent to the model.

I think we're fine to move to the next topic?

---

    Why not a JWT? It would duplicate the proof each op already carries

Fair, and thanks for the correction.  Yes, let's embrace the cryptographic truths that are native to the architecture.

    The JWT shape is still worth keeping. The Otto runtime can build a claims object from the verified op, compatible with Otto's JWTPayload, and hand it to handlers the way Otto does today

Ok.  Works for me.

---

We agreed on the form:

    carbon.call('POST [carbon]/listen', {
        to: 'POST [carbon]/members',
        in: ['POST [logger]/log'],
        out: ['POST [logger]/log']
        })

And now you're changing it yet again.  Do you have a reason for doing that?

---

    One limit: the tracking is per table, so a watched GET re-runs whenever its table is written

I thought you said CarbonDB should know specifically what fields GET pulls back... and it should know other filters like the tenantId...

    QueryRunner could narrow that to rows later.

Ok.  Maybe we just do that then.

In any case, I think this is a pretty safe and efficient thing, because we're querying in memory SQLite data.  Let's just not worry about it for now.

    1. When a sync delivers 500 ops at once, the watch should re-run once after the batch, not 500 times

Yes.

    2. Invitations and device joins could be GETs in the [carbon] namespace. Then there's no separate event API at all.

Yes.

---

