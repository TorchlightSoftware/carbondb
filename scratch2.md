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

    [carbon] is implemented in Rust, so its namespace is the one exception to "the core knows no routes". It needs keys and p2panda, and it has to work for the planned Flutter bindings too.

Why is that a problem?

---

    There is a real issue, but it isn't about Rust: a rewind can't undo what POST [carbon]/members does in p2panda. Undo only reverses SQLite writes, so re-running this op must not repeat the p2panda add, and rewinding it can't remove the member again

1) Regarding Rust implementation, I'm actually quite excited about this, it means the abstraction provided by CarbonDB is essentially a message bus that handles state management under the covers.  And being a message bus, it essentially allows pub/sub semantics and language agnostic handlers.
2) Regarding your issue: can you figure out how to make rewinds work with this?  We added this as a service call on your recommendation so that it would be part of the log.  What you're saying now is contradictory to your original recommendation.

---

    A message sent concurrently with the author's removal could therefore survive on a replica that got it first and vanish on one that got the removal first

The invarient the system MUST provide is eventual consistency.  The most straightforward way of providing that in this case seems like retroactively discarding writes that have arrived after the removal.  Then all the peers will match.

    Subscribers can be re-run: anything inside the transaction may only touch the database. Outside effects go after commit, as the reconciler does. Plain pub/sub doesn't have this rule.

Yes.  We will need to do something about object store requests, or other state changes outside of SQLite.  Basically all of our state changes need to have known repeatability semantics, and CALM rules applied.

    Language-agnostic applies per service, not per replica: a Dart [chat] and a JS [chat] in one space must behave identically, or replicas drift silently.

Yes.

    Add a critical topic on subscribers inside the transaction versus after commit

What does that mean?  A service call is basically considered atomic, right?  I don't know what "inside the transaction" means.

    Add a topic on keeping one service's handlers identical across languages

You don't need a topic on that.  Just document it.

I agree with your other proposals.

---

    The removal op carries a cutoff: the highest op the remover has seen from each of Bob's devices.
    - Rule: every replica discards Bob's ops above the cutoff, and rewind cleans up anything that depended on them. Peers converge because the cutoff is data in the log.

Yes.

    Proposal for repeatability

Brilliant.  I've added it to stories/README.md

    Question: does the cutoff work for you, given that it discards the removed member's unseen offline work?

Yes.  That is what MUST happen.

---

    Is the identity log a p2panda group, a separate log CarbonDB defines, or both?

We're trying to replicate how identities worked within KeyBase.  It's a group in the sense that it contains multiple devices, and the devices contain the assymetric keys that are used to sign messages and act as the roots of cryptographic authority.  And so the "identity" itself is a logical abstraction over multiple devices.  The "identity" then is the abstraction that we attach all other access control and custody to.

I'm unfamiliar with the exact semantics of groups and whether they are sufficient to support this abstraction or whether we require additional record keeping.

    New gap for the backlog story "A person removes a lost device": revoking a device hits the same p2panda drop as removing a member, but the revocation isn't an op in any space log

It MUST be an op.  And you're nitpicking about a story that we haven't defined yet.  I don't think you need to do that.

---

    My proposal: an identity is a small space of its own
    - Members: only that identity's own devices.

Mmm... We have `identity`, and as I said it's the main atomic unit for which we will be assigning membership and access.  But underlying that, we have the actual devices which carry the cryptographic proof of authority and of `identity` membership.

But I'm unclear on whether a `space` is correct to use for the `identity` -> `devices` relationship.  When `space` is also what we will be using for a "team context" which could be a company, a department, a team, a group of friends etc. and could be a chat, a project management board, a brainstorming app, or a sales CRM.

I'm unclear on the p2panda terms and their usage.  What's the difference between `groups` and `spaces`?  Can they be recursively applied?  Are they hierarchical or relational?  Maybe you can find some documentation on that to peruse, and give me a brief summary.

    - Ops: adding a device, for example, is a [carbon] op

This I like.

    - p2panda group: the reconciler keeps it matching, and other people's replicas use it to tie a device to an identity.  They never need your identity space.

Not sure.  More reference to p2panda literature would help, as above.

    Keybase's per-user key rotates and I found no single root key in its docs, so the id can't hash that

I want to go with keybase's model whenever possible because "they know what they are doing", they evolved their model out of user and scenario testing, and their model actually worked.  So I will prefer similar semantics to keybase unless we have a well justified reason for diverging.

    No server means no fork prevention: Keybase's server orders every change. Here, two devices that revoke each other concurrently are both removed (p2panda-auth/src/group/resolver.rs:35-41, same checkout). A thief and an owner racing would leave only the paper key, and I found no guard that keeps a last manager in place.

This is a place where we're diverging from Keybase: no server.  And I do want to stick with that.  But we should iterate the consequences of that design decision (not now, but at some point).

Regarding the "thief and the owner racing" I think you're referring to a context of personal device constellation?  There are steps that could be taken, for instance I have a YubiKey, and I could set that as the root authorizor for all devices, and also require it for removal of any devices.  And as you say, the paper key.

These aren't things we need to solve right now.  We're trying to "Clarify internals so that implementation can begin." and that means of the stories we have currently agreed upon.  This feels off the critical path.

---

    In p2panda's terms, an identity is a group
    groups nest, so a group's members can be individuals or other groups.

Agreed.  That makes more sense to me.  So you can refer to the identity as "this group of devices" and give it access to something, and all the devices get access.  Very straightforward mapping.

And I agree this is different from spaces which represent an active sharing context for data and collaboration.  We have also discussed that a "personal" space will be accorded to the identity as part of the bootstrapping process, but that's entirely additive to the abstraction we are talking about.  "identity" is atomic.  "personal" would be its own group + space, and that group would include the "identity:self" by default, but that membership is flexible and other identities could be added.  Identity membership in the "personal" space/group is completely distinct from device membership in an identity.

    Relational, not hierarchical: one group can belong to many groups and spaces, and a change to it affects all of them

Nesting is hierarchical, Claude.

    My revised proposal: an identity is a Keybase-style chain plus a p2panda group

Ok.

    Devices are a documented use: p2panda describes an actor as possibly "one of several devices controlled by a single person"

Wonderful.

    - Chain: the Keybase sigchain equivalent. It's a signed [carbon] log on its own topic stream, holding device entries, their names and the per-identity key encrypted to each device

Ok, this opens another question though:  What space should [carbon] events occur within?  Is there a hierarchical context to spaces as well?

    @system
    |_ @personal
    |_ @company-foo
        |_ @marketing
        |_ @engineering
    |_ @soccer-buddies

We may also want some notion of "heavy" and "light" spaces...  A user could have some devices that are assigned as part of their "heavy" group and other devices as part of the "light" group... Let's say they have a dedicated server in their living room, or they pay for a relay on Cloudflare.  These are heavy, whereas their mobile phone and laptop may be "light".  The light devices may still have access control enabled, but they access the services remotely.

This isn't something we need to dive into now but could be recorded as a story in stories/backlog/

    Two changes to follow Keybase more closely

Accepted.

---

    The one wrinkle is that a p2panda group can have several parents, so it isn't a strict tree

Interesting... What does that mean?  Draw me a graph.

    you didn't answer whether the chain is readable or encrypted

Hmm... can you add a security review story to stories/backlog?  I would rather we accomplish our initial prototype and then carefully consider and revise for security and privacy.

    - New critical topic: "Decide where [carbon] ops live and whether spaces form a hierarchy", including your @system sketch.

I think we should talk about that next.

    - Not done yet:

update the md files.  You don't have to ask for that.

---

    The Rust test in spikes/p2panda-demo is the only item left

What are you talking about?  That's really vague.

    A group with several parents makes a graph, not a tree

Eh, I see what you're saying, but I don't care.  Next.

    Decide where [carbon] ops live and whether spaces form a hierarchy

    Device entries go on the identity chain

Maybe that's an argument for the "identity chain" being an oplog, IE a "space" rather than just a table.

    p2panda groups nest, but nothing I read says spaces do

Yeah they shouldn't.  But I think we're uncovering cases where we want groups and spaces to closely track with each other...  maybe not always, but sometimes.

    Open question 1: What space should [carbon] events occur within?
    - My proposal: none globally.  A [carbon] op lives in the log of the thing it changes: devices on the chain, a space's own changes in that space. A shared @system log would need everyone in one space, which conflicts with having no owner and no server.

Ok.  So basically [carbon] is always a peer within the space.  I like that because it makes spaces atomic, not assuming any outside structure.  And I think it's more consistent with the usage we were assuming: services should be able to call [carbon] directly, assuming they have the proper access granted to do so.

And groups are already hierarchical, spaces don't need to be.  If Bob is removed from AcmeCorp (group), he gets removed from Marketing and Engineering (groups), and all of the spaces that were granted are now revoked.

These aren't separate stories: they affect the internal implementation of the stories we have.

    - @system can still exist as the local system database: identities and the catalog, on this device only, with no log and never replicated.

I'm not sure.  Would need to review use cases and consequences.

    Open question 3, new: Which record tells a newly added device which spaces its identity is in?

Is that still relevant?  Bring it up again if so.

---

    The Rust test, explained: this is the check that replicas can't end up holding different ops.

Add that to the backlog, I don't want to talk about it now.

    Removing Bob from AcmeCorp doesn't reach Marketing
    - Bob is in both directly: he leaves AcmeCorp and stays in Marketing.
    - Removing him from Marketing is what flows upward.

What?  That's pretty bizarre, you realize that right?  Are you understanding p2panda correctly?

    My proposal: removal from a parent is a [carbon] behaviour. It publishes one removal op, with its cutoff, into each affected space. That gives you your cascade and solves the cutoff gap, and each space stays atomic.

I'm fine with that.

    I agree the identity should have its own space, running only [carbon]

What does it do?  Describe typical Op lifecycle in association with the stories we know about it.

    Groups and spaces tracking each other points to a simple rule
    The log decides membership, and only spaces have logs.

Hmm... I think it's generally true, but I'm not willing to say "always true" because I don't understand the possible scenarios we will encounter.  There are probably reasons why p2panda made groups and spaces separate concepts.

    "Which spaces is my identity in?" is still relevant, and the identity space answers it

    A new device needs space ids from somewhere. It joins the identity space, syncs it, and reads an entry for each space joined. The encrypted entries I proposed are no longer needed.

Sounds like a good fit potentially.  Let's review the "identity space" lifecycle as requested above.

---

    You were right to doubt it: I got the Bob example wrong. If Marketing is inside AcmeCorp and Bob is in both directly, removing him from AcmeCorp leaves him in AcmeCorp through Marketing, because p2panda counts every member of a child group as a member of its parent

You're contradicting yourself.  What you mean to say is you were right about p2panda's behavior.  You don't have to lie to appease me.  Speak the truth.

    p2panda's rule itself is sensible: removing someone takes them off that group's direct list and never walks down into child groups. Your agreed cascade fills that gap.

Ok... is it always appropriate to cascade?  It's appropriate in business context: "Bob left the company."  But maybe it's not appropriate in all contexts.

1) should cascade: always be implemented at CarbonDB level?
2) or should it be configurable?
3) or should it be left to the application/services to implement if they desire it?

Otto would implement this certainly - it's desired logic within that domain.  What are the scenarios where one might not want this behavior?

    identity.call and identity.watch become plain calls on one more space, so GET [carbon]/devices and GET [carbon]/invitations need no special case.
    The per-identity key no longer needs encrypting to each device, because only your devices can read this space

Those are good things.  I agree, and I like this.

    creating a space writes to the new space and the identity space separately

I don't like that.  I don't understand why it's necessary to track in two separate places...  This is like double entry accounting or something.  And I guess it gives you pointers in both directions?  Justify.

This is also starting to look like the hierarchical spaces that we said we don't need.

    @system
    |_ @identity:Brandon
        |_ @team:BuildIt
    |_ @identity:Sally // second identity on the same device; shared device use does not imply conflated identity

IE: you're putting @team:BuildIt join and leave messages within @identity... it's not a hierarchical relationship in the "CarbonDB ecosystem" sense, but it is a hierarchical relationship on a single device.

Open my CarbonDB app: it wants to know what Identities do I have?
Open my Identity: it wants to know what Spaces am I in?
Open a Space: it wants to know who is a member?  Are there any sub-spaces?

    listIdentities still needs something local, because it runs before any identity space is open. That's one concrete use case for @system

Ok.  @system being canonically local.

---

    Cascade should be left to services (your option 3)
    It's a policy about what a hierarchy means, and it's sometimes wrong:

In all the cases you've mentioned:

1) it would probably be acceptable to kick Bob, and then let him rejoin
2) people can also choose to set up groups outside a hierarchy if they wish them to be independent

I think that this should be a configurable setting on CarbonDB, default: ON.

    the identity space holds only a pointer: "my devices should open this space"

Mmm... then I wonder if it shouldn't be a separate [carbon] call.

    [carbon]/space/join   # join the current space we are in
    [carbon]/space/leave  # leave the current space we are in
    [spaces]/add  # maybe spaces record keeping should happen entirely independent?  In a different service?  Optional service, but bundled with CarbonDB?  Or an app can elect not to include it, but write their own?
    [contacts]/add # same with contacts... it's a pointer to a bunch of carbondb stuff, but could contain meta data, we could provide a default implementation but apps may want their own

The pointers are really serving a different purpose which feels more like it belongs in the application: to render UI for accessing the space... maybe it needs meta data, group description, etc... CarbonDB shouldn't necessarilly make assumptions about those things?

---

    The cascade setting has to live in the log, not the TOML

Should probaby be a field on the initial create Op, right?

    The pointer does two jobs, and only one is UI

Ok.  Then let's take responsibility for it in CarbonDB, and do our best to capture some basic fields for metadata/application use.  But knowing that we don't have to be comprehensive: the listening system lets an application track 1:1 with our ops and extend with additional data if they wish.

    it's a property of each group, recorded in that group's [carbon] log and defaulting to on

How are groups synchronized among p2panda peers?  Are we duplicating inherent group functionality by putting mutations on the group into an Oplog?

I like the idea of "oplog all the things" on the one hand... but I just want to think through the consequences and make sure we're not resisting or duplicating what the underlying framework already does for us.

    join and leave can't go in the space being joined

I need to see examples of scenarios laid out, with all the oplog calls.  I don't want to try and figure out what you're saying without examples in front of me.  We need to be able to create something like this:

    @device:Pixel9 (we should probably do this instead of "system")
    |_ @identity:Brandon
        |_ @company:FooCorp [identity, chat, books]
            |_ @team:Sales [crm] -> (tom)
            |_ @team:Marketing [website, sms, automation] -> (sally)
            |_ @team:Service [worklog] -> (hank, forest)

What would that look like?  For creating the spaces with the services, and inviting members.

---

    My proposal, which reverses "the log decides, p2panda follows":

This is hard for me to follow, I don't understand what you're trying to say.  Can you be more specific and show an example with code and ops?

    Groups can't hold Manage

This goes back to: I think an Identity needs some way of tagging the devices: which then gets translated to how they get treated in different scenarios.  I mentioned "light" and "heavy" before.  There could also be "manage".  Maybe these "tags" would be implemented as groups as well?  For draft purposes we could bootstrap the initial device with "manage" priveledge.  But regardless, I think CarbonDB itself needs a way of taking a "manage" group and applying "manage" permissions to everything in that group.  So we're effectively implementing group management on our side, and letting the Identity make that decision once, and apply it across all their spaces.

    Your tree mixes directions: Pixel9 is a member of Brandon and Brandon of FooCorp, but Sales is also a member of FooCorp.

    Your tree as ops

I'm really having trouble understanding what you wrote here.  It feels like I'm speaking Greek and you're responding in Spanish.

1) follow my lead on `@device:Pixel9` format for a space name.  You're flailing around searching for a format when I gave you one you didn't use.

    FooCorp group
    ├─ device:Pixel9     Manage
    ├─ identity:Brandon  Write
    ├─ Sales group       Write
    │  ├─ device:Pixel9  Manage
    │  ├─ identity:Brandon Write
    │  └─ identity:Tom   Write
    ├─ Marketing group   Write    (same, with Sally)
    └─ Service group     Write    (same, with Hank and Forest)

2) I see what you're getting at with this tree but god that's obtuse.  I gave you a tree that had order of operations implicit to it, and some nesting in terms of where the commands would be run, and you gave me a tree describing group nesting and permissions.  We're not speaking the same language.
3) You start with `identity.createSpace({name:'FooCorp'})` but you're calling an identity that doesn't exist yet.  Start from 0, new device.

Please let's just talk about the tree and order of operations and sideline other topics until we agree on that.

---

    How I read your tree

    - @company: and @team:: the prefix is a kind field on the create op.

Yes.  Setting the meaning of "kind" to the side for now...

    - foo.createSpace(...): calling it on a space makes the new space its child. Story 6 only has identity.createSpace, so this adds to it.

To clarify:

    @team:Marketing is a child of @company:Foo
    but @identity:Bob is not a child of @team:Marketing, he's a member
    adding @identity:Bob to @team:Marketing DOES NOT add him to @company:Foo, and depending on the rules of @company:Foo, may or may not be allowed

I'm wondering if you're applying groups backwards, and if that is why all this confusion is arising?

    - Brandon is in every team: he created them, so createSpace adds him as a member.

Yes.

    @device:Pixel9
      carbon = await configure('./carbondb.toml')
      {identity: brandon, paperKey} = await

Now you're mixing javascript and Ops shorthand.  Let's stick to ops... we need a way of describing what channel we're running the ops in.  @device:Pixel9 format does that.  Stick with me on this and just show the ops.  Can you do that?  And if there's something specific you want to tell me about Javascript land, put that in a separate block.

---

    p2panda nesting means "every member of this group is also a member of that one"

"this one and that one" is not helpful language Claude.  No part of what you're saying clearly expresses the relationship between p2panda code and the semantics we are looking for.  I'm going to wait for you to clarify this before discussing it further.

    The ops

Good.  You did a much better job on this.  This tracks with the message bus concept that we are working with here, it starts to look more like a plausible set of console.log messages that you would see if you turned on debug mode.  I saved the file to stories/oplog-examples.md.  Further feedback:

    @device:Pixel9 -> @system

I changed this back to system as the root space.  I hear you on @device being nested under Identity.

    slug:'Brandon'

In general slugs should not have upercase characters.  If you want to operate purely on slugs for example purposes, then use slugs that are properly formatted with dashes.

  @identity:Brandon
    POST [carbon]/devices {name:'Pixel9', tags:['manage']}

Device registration is a POST in the @identity.  Makes sense.

1) do all spaces support adding devices like this?
2) should there be an [identity] service?
3) if it were a separate service, then we could stop it from being added in spaces where it does not apply
4) also the [carbon] namespace wouldn't be polluted
5) if it remains on [carbon] perhaps it should be `POST [carbon]/identity/devices`

    POST [carbon]/key {per-identity key}

Same with `/key` should be nested under `identity` path or separate service.

    POST [carbon]/space/join {space:@company:FooCorp}

Seems like this should be a create, not a join?

    POST [carbon]/space/join {space:@team:Sales}
    POST [carbon]/space/join {space:@team:Marketing}
    POST [carbon]/space/join {space:@team:Service}

Hmm... you're putting these inside the @identity because they are record keeping for the @identity?  These feed the "pointers" that power UI and lookups to know what spaces we are in?

    @company:FooCorp
      POST [carbon]/space {kind:'company', name:'FooCorp', services:['identity','chat','books']}

Ok, so @company contains its own creation, as the initiating log entry.  The only thing I find problematic here is that I have to create the space out of band.  I have to call Javascript code (and the underlying Rust) in order to create a space.  I can't do it directly from an Op.  And I think that's problematic because you lose flexibility, and you lost record keeping.

I would rather see:

  @identity:Brandon POST [carbon]/space {kind:'company', name:'FooCorp', services:['identity','chat','books']}

  @company:FooCorp POST [carbon]/self {kind:'company', name:'FooCorp', services:['identity','chat','books'], parent:null}  # Brandon is not an implicit parent of FooCorp

  @company:FooCorp POST [carbon]/space {kind:'team', name:'Sales', services:['identity','chat','books'], parent:@self}  # FooCorp is parent

IE: there is an external parent context which creates the new space, and then the new space gets a (spawn, self, origin, seed, founding... don't know the right term, what do they say in blockchain parlance?) Op which carries its instantiating properties.

The issue with putting the instantiation within the space itself is: where does it come from?  Who calls it?  What scope does it implicitly operate under, if any?  When you don't put that within a parent scope, then you rely upon it happening out of band, in JS or RS, not as an Op.

      POST [carbon]/children {space:@team:Sales}
      POST [carbon]/children {space:@team:Marketing}
      POST [carbon]/children {space:@team:Service}

Parenthood should be established at the moment of creation.  Reassignment is undefined, not something we have talked about yet.  Can be added to the agenda, not critical path.

Can you reframe now from this perspective?

---

    Your approach works: a parent creates the space, and the new space's first op records its own founding.

Thank you.  That feels much more comfortable, and mirrors... lexical scope I think it is called?  @system operates like main() and then is able to reference and coordinate between the things that it is creating.

    The blockchain term for that first op is genesis, so I've used POST [carbon]/genesis

Eh, I don't like it.  Let's use "origin".

    @company:FooCorp POST [carbon]/origin {kind:'company', name:'FooCorp'}

Sounds good to me.

    p2panda has one call for adding a member to a space, and the member can be a device, an identity or another space's group:

Are groups always attached to spaces?  Can it be a group of groups with no directly associated space for instance?

    So adding a group grants access to everyone in it

Ok.

    whereas you want Sales under FooCorp

Yes.

    while Tom stays out of FooCorp.

Not sure I follow.  In what scenario do I want Tom to not be in FooCorp?  That's possible in some other example (e.g. Tom is a consultant) but that wasn't what I was trying to spell out in my example.

But your flow now makes WAY more sense to me.  Please update stories/oplog-examples.md

    Do all spaces support adding devices?

I think segmenting what services are running in the space is the cleanest way to do that.  So the `origin` op clarifies completely what is "in scope" for the space.

    Should there be an [identity] service?

Let's just make sure we're very clear that this is different from the [identity] service in Otto.

    Your design needs three rules

    1. Only the author's device creates the space.

Within the service-key spec, and within Otto plans (accounting/data-custody), we developed the concept of a self custody service as `[@service-name]`.  We could apply this to CarbonDB.  @system is local, [identity] is not.  However, we still run into a hitch: we would still be creating `@company:Foo` from within `@identity`, and `@team:Sales` from within `@company`.  We could resolve this by saying that all space creation (and any other CarbonDB commands which don't need to be replicated) are performed within the @system space (probably not a space per say, it's an Oplog but it's unreplicated... could be implemented as a space with a single device/member if that's most straightforward.)

    2. The space id is the hash of the create op.

I'm not opinionated about this right now.

    3. A rewind can't undo a created space. Creation belongs in the Irreversible class of the repeatability table, like sending an email.

Ok... that means something that I can't fully understand right now.  Noted.

---

    My proposal: @system does the work, and the parent records it

Hmm.  I don't know if you're gaining anything from that.  A replicated @identity would still replay a @system sub-call on another peer, it would just talk to the local system there.

I think we have competing desires:

1) record of a space should transfer to all devices within an identity which "sees" that space
2) the space creation itself is a call to CarbonDB/p2panda, and it's already internally replicated, you don't want to replicate the creation and perform it on all devices
3) desire for the call to be visible in the op log, and callable from anything with access to the op log

What would help us to get clear on "the right design" for this?

    Create spaces with [@carbon] in @system, plus a [carbon] record in the parent

Wow.  I'm realizing how confusing it is to use @ symbols both in service names and in space names.  Let's drop the local service names nomenclature for now, it's too confusing.

---

What if we do:

    // this creates the new space in CarbonDB/p2panda
    @system POST [carbon]/space {kind:'company', name:'FooCorp', parent:@company:FooCorp}

    // internal implementation calls `ref` in @identity
    @identity:FooCorp POST [carbon]/ref {kind:'company', name:'FooCorp'}

    // and `origin` in @company
    @company:FooCorp POST [carbon]/origin {kind:'company', name:'FooCorp'}

1) I think you need the ability for Ops to happen that don't replicate, if you want to turn system calls like space creation, and probably the rest of p2panda calls, into ops.
2) The local Op implementation gives you a place to wire up the subcalls
3) this gives you logging, listening, and call capabilities

---

    @system ops never replicate,

Yes, we've beat that one to death.

    and the ops they call are records, so replaying those is harmless.

I'm not sure that's true.  Why would it be?  They're just normal ops.  Replaying all ops should be harmless.  Don't nag me; I know there are exceptions if we do some kind of non-SQL state manipulation that we don't have a rewind function for.  My point is there are no special rules for what a @service can sub-call out to.  ALL Ops are already designed to function in replicated form.

    The missing rule: calls only flow outward from @system

Painfully obvious, but true.

    Together with your design, that gives one simple rule: [carbon] in @system acts, and [carbon] in any other channel only records.

Garbage.  Mumbo jumbo.  Nonsense.

Have we established that [carbon] should BE anywhere except @system?

    as:@identity:Brandon

We're providing cryptographic identity implicit in this whole system.  You don't pass it as a field param.

    ref needs the space's id

Yes.

    The parent also gets a ref:

Uh.  The parent was my main use case for ref.  You seem to be putting the ref to Sales in both FooCorp and Brandon, which I think is unnecessary.  To me that would mean: Brandon has access to Sales through his FooCorp membership, and he also has direct access to Sales, even if FooCorp membership is removed.  That would be like if I'm both a consultant and an employee: it's possible, maybe, but it's not the use case I was intending to outline.

Your example switches back and forth between prepending the space before the op, and not prepending it.  Which is annoying and hard to read.  Use my format: prepend the space always.  That way it reads like a debug log would, each line is independent and doesn't require the others in order to read.

    - The parent's ref runs first: if FooCorp's rules refuse it, nothing gets created.

Maybe.  I see what you're trying to do:

    @company:FooCorp POST [carbon]/space

Would have been a more direct way for the company to apply policy to the creation of spaces.  When we do:

    @system POST [carbon]/space

Then we're at a loss in terms of "how do I authorize that?"  What you're saying about authorizing the ref might make sense though: of course anyone acting in their own autonomy is welcome to create any spaces they wish.  But if they wish to attach that to the company space, to have it visible to members of the company, and owned by the company, then the ref must be accepted first.

That sounds pretty fair to me.  And it sounds like it would be straightforward to implement in the @system [carbon] handler.

    Are you OK with a replicated handler never creating a space directly?

Let's say that I'm ok with it for now.  And we can explore it further later if we want to create some way around this.  I think this is already true in the way that Otto handles local services - they cannot be called by any of the networked services.

---

    "[carbon] is a peer in every space" and stories 2, 6, 7 and 9 all predate @system ops

Exactly, thank you for recognizing that.

    Replicated spaces: I'd give them a core [space] service that runs in every replicated space

Great.

   alongside the core [identity] service in identity spaces.

Great.

    The ops, as a debug log

Hmm.  That is really hard to read.  Thanks for trying it.  I like your nested form the best, like:

    @system
      POST [carbon]/identity {slug:'Brandon', device:'Pixel9'}

That looks better.

    - One ref per space: it goes in the parent, or in the author's identity log if there's no parent.

1) What if instead of "parent" we say "owner"?  And we set 'cascade' to 'add', 'remove' or both?

    @system POST [carbon]/space {kind:'team', name:'Sales', services:['crm'], owner:@company:FooCorp, cascade: ['remove']}

Cascade has to make it through to the ref:

    @company:FooCorp POST [space]/ref {space:@team:Sales, kind:'team', name:'Sales', cascade: ['remove']}

And owner makes it through to the origin:

    @team:Sales POST [space]/origin {kind:'team', name:'Sales', services:['crm'], owner:@company:FooCorp}

2) if there's no owner set, then no one has the right to delete the group or add members.  When all members leave, the group dissolves.

3) if cascade: ['delete'] is not set then the deletion of the ref, or of the space holding it, does not cause the origin to be deleted.

4) if cascade: ['delete'] is set, but owner does not match, there's probably a permissions error when this is triggered, unless "manage" rights were assigned at some point.

5) we don't need to get into the weeds any more, I think this is sufficient for first draft.

    It sees all three teams, but Sales' member list sits inside Sales, so it can't tell which teams it belongs to

What?  What are you talking about?  Why wouldn't it be able to read this from Sales?  Remember, there's also two separate notions: membership and access.  membership is a p2panda concept, and I think the fact that we are in the group should be verifiable... Do we need:

    @system
        GET [carbon]/spaces
        GET [carbon]/spaces/@team:Sales

Right?  We have to be able to read from the base API, not just write to it.

---

    A device that subscribes learns the space and its members without being a member

Mmm... that sounds not helpful and possibly misapplied.  We don't want to reveal space membership to anyone who asks.  You need an invite.

What problem are you trying to solve Claude?

    FooCorp needs an owner

Fair.  Ok.

    A space has one owner but can have many refs

Hmm... are we right to track it as "owner" or maybe it should be "manage" and should be a list?  To track with p2panda's concept?  Or maybe it doesn't need to be tracked at the application layer, because:

    @system
        POST [carbon]/spaces/@team:Sales/manage
        GET [carbon]/spaces/@team:Sales/manage

Is a thing?

    A space has one owner but can have many refs

Yes.  I thought this was obvious.

    POST [identity]/devices {name:'Pixel9', tags:['manage']}

This should probably be:

    @system
        POST [carbon]/devices {name:'Pixel9', tags:['manage']}

    @identity:Brandon
        POST [identity]/ref {kind: 'device', name:'Pixel9', tags:['manage'], pubkey: 'asdf942'}

Or something like that.  The @system call needs to generate public/private key pairs, and it doesn't get replicated to other devices.  The ref is what authorizes the key to act on behalf of the identity.


---

    The problem I'm solving: Tom adds a second phone, and that phone has to find out which spaces to open.

Jesus.  That was pretty unclear.  Shouldn't they be refs inside the @identity?

    @identity:Tom
      POST [space]/ref {space:@company:FooCorp, kind:'company', name:'FooCorp'}
      POST [space]/ref {space:@team:Sales, kind:'team', name:'Sales'}

    You objected to this for Brandon because

No, I objected to that because I want you to iterate through the tree, not just store everything at the top.  Tom is a member of FooCorp.  He needs to look in FooCorp to find the refs to the individual teams inside.  I don't want them to be dumped directly inside his identity.  Because he doesn't have direct access - he has access THROUGH his employment at FooCorp.

I don't know why you're misunderstanding this, and making me repeat myself so many times.  Please knock it off.

    p2panda only stores individual devices with Manage, and can't store "FooCorp manages Sales"

Ok.  Sounds like we have our own logical "manage" permission, and we need to then map that onto p2panda.  That's fine, and we don't need to discuss all possible scenarios right now.

I also don't think cascade: ['remove'] has any sensible meaning if we're using refs the way I think we are.  If I delete the parent entity, what does it mean to cascade a remove?  The ref is already going away, there's nothing for it to exist on any more.

    - The two refs behave differently: [space]/ref only points, while [identity]/ref grants authority. The service name keeps those apart.

If you have a ref that grants authority you need to either: A) annotate it to make that explicit, or B) use a different service call.

What about {kind: 'manager'}?  Or should we track with the different p2panda permissions: {permission: ['pull', 'manage', 'read', 'write']}

Of course that's implicit to the space itself, but maybe it's convenient to have record of it on the reference.  Or maybe we don't because the reference should be clean of knowledge of this sort of thing?

    Proposals, pending your OK

    1. Every member keeps a [space]/ref in their identity log, written when they create a space or accept an invitation.

Ok.

    2. Replace owner with a manage list on the origin op, kept in the log and expanded to devices by CarbonDB.

Ok.

    3. Move member and manage calls to [carbon]/spaces/:space/....

Yes.

    4. Add "a subscriber holding a space's id can read its member list" to stories/backlog/security-review.md.

Let me repeat myself: "that sounds not helpful and possibly misapplied".  That means that's a dumb idea Claude, please don't try and sneak it in as gospell.

Update the agenda, and the oplog-examples.md.  You can add your second person joining the teams, and then adding a second device.  Update the stories and stories/README if need be.

Report back with what's still undecided/unclear.

---

    Looking in FooCorp shows it Sales, Marketing and Service, but not which ones Tom belongs to.

Ah, now I see your point.

    My recommendation is that FooCorp's log records team membership, since teams belong to the company and only its members can read that log.  The cost is that every FooCorp member sees every team's roster.

Yes, this is a better solution, because the data lives with the company, not with Tom's personal identity.

    Does removing Tom from FooCorp also remove him from Sales?

Yes.

    Your earlier decision says yes, by default. But with cascade now delete only, nothing does it.

Originally we had 'remove' and it meant "remove Tom's access to Sales when his access to FooCorp is removed".  I guess maybe 'delete' means "delete Sales when FooCorp is deleted"?  That's fair.  So I guess all the teams would need {cascade: ['remove', 'delete']}.  I just didn't understand the meaning of what you had newly added and we never stopped to clarify.

    Should a device use a ref that grants authority?  You gave two options, and I chose a different call: POST [identity]/devices {name, tags, pubkey}. Refs then stay plain pointers with no permissions on them, which also answers your "keep the reference clean" question. Permissions are read from the space itself.

1) p2panda already has a concept of authority in terms of [pull, manage, read, write]
2) Otto (and theoretically other service frameworks) have a sense of role and feature based access controls, which should be able to be ported into a CarbonDB context relatively intact

There are two questions:

1) Is there additional record keeping that needs to be overlaid on top of the p2panda [pull, manage, read, write], in order to display on interfaces or expose controls?  Or can all of this be accomplished by @system calls that directly query the p2panda data?  Cases for our own system that we've brought up so far:

    A) overlaying manage onto groups
    B) calculating @company -> @identity -> @device membership
    C) applying company policy to group membership
    D) maybe something I'm missing?

2) How does the RBAC [identity] system of Otto interplay with a system like CarbonDB?  Otto defines feature access based on service-keys.  This was designed more for the "remote access" scenario, but I think it meshes well with the [pull, manage, read, write] permissions.  [pull, manage] are orthoganal to the RBAC.  [read, write] overlay with RBAC: you need both READ on the service, and access to a particular GET route in order to read.  You need both WRITE on the service, and access to a particular PUT, PATCH, POST, DELETE route in order to write.  The services get a JWT from [identity] which describes all of their permissions.  And the service logic itself (middleware) enforces this access control.  So the Otto RBAC seems to layer well with CarbonDB:

    A) it can be made compatible with the crypto keys, proofs, signing for identity and devices
    B) should be compatible with [pull, manage, read, write]
    C) technically with [pull] access you're being trusted to host everything, so you have direct access to the data if you want it, but peers will still reject write transactions that you're not allowed to do
    D) RBAC could probably be looked at as an optional module in this system: without it you're still going to have strong identity and the broadest sense of permission management, you just won't have per-service-key granular permissions.

You were originally asking "Should a device use a ref that grants authority?"  I think that it is probably unnecessary, given that the spaces themselves record this, and then RBAC does its own thing.

But:

    POST [identity]/devices {name, tags, pubkey}.

You're inferring ownership from this...  If we apply the same logic as above:

1) device private keys, maybe some other info sits in system
2) device metadata, public keys, authorizations, sit in @identity

What if we had:

    @identity POST [device]/origin {name, tags, pubkey}.

It's just a little bit different than what you had.  But it tells me: 1) this was instantiated in a different space, and then ended up here, 2) [device] is its own service, with whatever operations belong there

    Which JavaScript handle makes @system calls? Story 7 now uses carbon.call, but carbon doesn't say which identity is acting. I'd use identity.call, which needs a way to reach @system as well as the identity space. I'd settle that with the first implementation.

1) I'd say 1:1 between spaces and handles in JS/RS.  So in our present sketch we have:

    system
      identity:Brandon
        company:FooCorp
          team:Sales
          team:Marketing
          team:Service

So perhaps you could go:

    // carbon = system
    carbon.call("POST [@carbon]/identity {slug:'brandon', device:'Pixel9'}")

    // all other spaces available through refs
    const sales = carbon.ref('identity:Brandon').ref('company:FooCorp').ref('team:Sales')
    sales.call("POST [chat]/room/:id {id: abc123, message:'hello'}")

Note that I removed the @ symbols from the space names.  I think we should do this with the nomenclature, the @ symbols aren't helping, and they confuse with [@local-service] nomenclature.  Removing them clarifies something for me:

We could actually have [@carbon] operate in (potentially) every space.  And it's always local, but operates in the context of that space.  So if you add a space, it's naturally going to be nested in the space from which you added it.

    Undecided on the other two critical topics

    5. Membership and access:
       - Topic 7 still says "the log decides, p2panda follows", with a member table and reconciler. The examples now work the other way, through @system, so that agreement needs replacing.

Agreed.

       - It also needs to settle who may add members and which p2panda access each member gets.

This is already covered in the access controls of the space itself.  Isn't it?  And every call runs in context of a space.

    6. Invitation delivery: what carries an invitation between two people who share no space, what the QR holds, and whether a member must be online when someone accepts.

1) what carries an invitation is purposely undefined.  The invitation itself needs to not care about this, to be representable on different transports and mediums.
2) we may at some point build services on SMS, email, facebook, twitter, whatsapp, matrix, etc. that will transmit invitations.
3) we may assume for now QR code, and copy/paste invite code into medium of choice

Your last question, more deeply is regarding networking and accessibility.  For p2panda to work from one private network to another... does it require some kind of relay to be running, to punch holes through the firewall?  In that case, maybe all we need is to be connected to the same relay, and to know each other's public keys?

Please update the agenda.  Do not send me responses to all the things I've written.  Instead talk to me about ONE thing that you deem to be priority, and address the rest of the agenda ONE topic at a time until we have completed critical path.  Thank you.

---

    You've now suggested [@carbon] could run in every space: always local, but in that space's context.

Yes.

    1. Does [@carbon] run in every space, replacing [carbon] in system and the [carbon]/spaces/:space/... paths?

I think so?  What is on the @carbon service?

    My proposal: yes, adopt it. It brings back the lexical scope you wanted, since the handle says where a call runs.

I think it also opens up a clear meaning to local services...  They are accessible from any space, and they can call replicated services, but they can't be called by them.  That makes them a sort of local scratch pad... I could have a local service that runs on my phone and interacts with a business context and performs automations on my behalf... without directly installing that in the business context.

    Authorization has a natural home: a call in company:FooCorp runs under FooCorp's rules, which decide whether its ref is accepted.

True.  And a company that wants to ban local services they don't know about or approve can do so.

    Local ops share one log: I'd keep them in system, with each row naming its space. That gives one debug log and one place to finish interrupted work after a crash.

Hmm.  That's a fair interpretation for now.  I'm ok with it.

Yes, I'm in agreement with all of this.  Please update the stories and agenda, and pop up whatever is next to talk about.

---

    What's on [@carbon]: anything that must happen once, on one device.

Can you document the known APIs of all builtin services (@carbon, identity, device, anything else?) and put it in `stories/services.md`?

    I named the op for your team membership decision POST [space]/roster.

I don't know what you're talking about.

---

    What "roster" means: it's a name I made up for something you already agreed. Tom's new tablet looks in FooCorp and sees three teams, but it needs to know which one Tom is in.

Yeah, but I don't know why we're doing that in CarbonDB.  Is that something we need to spend time and attention on right now, or can we leave it up to the services that are running in the space?

In my previous example:

    const sales = carbon.ref('identity:Brandon').ref('company:FooCorp').ref('team:Sales')

Accessing sales should require that both: 1) the refs exist, 2) Tom has access to the underlying spaces.  This could be [pull, manage, read, write].  And what is possible with the ref at that point will depend on Tom's access.  I don't think that CarbonDB should be responsible for anything more than this.  I would get rid of the roster concept.

---

You're missing [identity]/origin in services.md.

POSTing anything to [space] should require MANAGE permissions on the space.  You agree?

POSTing /origin a second time should be disallowed.

    1. A device syncs a space only once something reaches it.

You presume that, but that depends on our implementation...  I would say add a config.toml entry to control that behavior.  Default do not sync until a ref is accessed.  CarbonDB instances with manual sync can also turn off sync, and delete the data.

    That's a change from "membership decides hosting"

This also depends on p2panda's internals, so I don't know if what you're saying is accurate.

    2. A handle can't always answer "no" right away. p2panda has no quick way to ask. Being added resolves joined, but for a non-member it never resolves.

Companies, or anyone who's serious, will have one or more relays with ['pull'] access.  But yes, we need to have defined behavior in the case of network partition.

Can you elaborate on how this is supposed to work in terms of p2panda?  And suggest some kind of reasonable mapping onto our semantics?

    My proposals, pending your OK

    1. .ref() returns a handle straight away, and GET [@carbon]/access answers pull, read, write, manage, none or unknown. unknown means the space hasn't synced yet.

Make it GET [@carbon]/status.  Return whatever metadata is appropriate for the space and it's status on the local device.

    2. Devices sync on demand: a space syncs the first time something reaches it, and stays synced on that device from then on. An app that wants everything on a device can walk the refs itself.

See above.

---

    GET [@carbon]/status

Can you just have a field `access` with [pull, manage, read, write]?

It seems like you're trying to chop this up into multiple fields and I would say: don't do that.  Just mirror the format that p2panda is already setting.  You can accomplish the equivalent of none by [], and unknown by a separate field, the sync status.  Or set access to `undefined`.  Yay.

Does status know how many peers have the space, and whether they were recently online?

    p2panda's relay server only forwards encrypted traffic and helps nodes connect

Fine.  I don't need to reinvent that or talk about it further right now.

    on-ref syncs a space the first time it's reached, and it stays synced on that device. In manual mode, POST [@carbon]/sync starts syncing and DELETE [@carbon]/sync stops it and deletes the data.

1) make it just "ref"
2) separate DELETE and STOP

Ok, update stories and agenda.  Can we make our way towards actually implementing this first draft?  What remains on our critical path?  I'm weary of answering questions and reading giant text walls.

---

    Under your model, single-node is itself a CarbonDB mode, with the same handles, call, watch and builtin services, and with p2panda left out.

Yes.  So the "main()" function of the JS process would load CarbonDB, and init the services with it.  And swapping the app to run in replicated mode would be a config change, not a refactoring of the app.

    execute(sql, params, mode) is a lower layer than the app interface, so it doesn't conflict with this

Right, but it's important to focus on what the app interface IS: that's the level at which our spec needs to be defined.  Other implementation details can shift in order to suit the internal needs as the buildout evolves, as long as our API and its gaurantees are maintained.

    A. Rust core with no network (my lean): the core still runs the op log, the shared order, sessions, undo and the builtin services.

Yes to Rust core.  But for expediency, and for focusing on getting the Node.js services built, you can drop the op log.  What observable end user behavior is it producing?  Is it important for a "Hello world" that gets the Node services built in the right shape?  I don't think so.  Look at what the Otto service abstraction is doing: binding services to HTTP.  We need to accomplish the same thing in single node mode, and then our tests will interact with it through HTTP.

One question we haven't really answered: When a node wants to listen to HTTP, is it CarbonDB that facilitates that, or does the service/runtime implementation do it itsef?  My thought at this time is CarbonDB should focus on being the replication, and HTTP is a separate concern.  Services that are bound to HTTP will therefore just be forwarded to the CarbonDB message bus with space.call(), and we don't need to have implementation of HTTP or any other protocols.  But I'm open to other perspectives.

Listening to HTTP will be an important consideration even long term with CarbonDB: there are cases where you want data replicated amongst peer set X, but peer set Y (no intersection with X) nevertheless has cryptographic identities that have been approved for remote call access.  This allows more fine grained READ control which is not possible if you just replicate all the data to their devices.  But the degree to which this is a CarbonDB concern... maybe not at all?  The RBAC [identity] service is an optional plug in, which CarbonDB is not aware of or has any knowledge of.  And the same could be true about remote access scenarios like HTTP, gRPC, etc.

But a reason to change that perspective would be if:

A) it's very valuable to have HTTP services standard from a testing, debugging, control plane, integration perspective
B) the tech can basically be considered "free" - there are good standard implementations in Rust.  You already have the services defined.

But it does presume that everything is bindable to HTTP.  The service-key definition doesn't make that assumption: HTTP method is optional.  Services can be defined for local access or gRPC which don't have any compatibility to be run as HTTP.

What do you think?  Shall we put HTTP bindings in CarbonDB, or leave it out?  I still lean towards leaving it out.  It's probably just a few lines of code in javascript if they want to bind services to HTTP.

    Open question: can a single-node instance turn sync on later?

No.  And furthermore: Your job right now is not to anticipate all possible future activity.  Your job is to get a hello world built with the correct application-to-CarbonDB boundary.

---

    Serving a remote caller means deciding who that caller is.  That's RBAC territory, which we already keep out of CarbonDB.

This is the strongest and clearest reason.  Thank you.  We definitely need to document that decision to make it clear to service developers why we set the boundary as such.  "Not every service key has an HTTP method" is also a secondary consideration which should be presented, but is less compelling.

    The adapter needs one guarantee from call: errors that map to HTTP status codes. If call rejects with an error that carries a code (validation, not found, forbidden, conflict), the adapter stays a few lines. Without that, every app invents its own mapping.

Hmm.  We already have something approaching canonical definition of error codes.  There are duplicate implementations in MicroQL and Otto.  Maybe intersecting, maybe conflicting?  Perhaps this should be split out as a @tlsft/service-error module.  Then the CarbonDB core can gaurantee that errors of this form can be serialized and rehydrated with the HTTP codes and other meta data preserved.

    The adapter, not CarbonDB, decides which space a request goes to

Yes... This is another STRONG reason for not automatically mapping routes to HTTP.  CarbonDB has internal nesting and what essentially amount to virtual networks without any attachment to transport protocol, network boundaries, or IP addresses.  So the decision to bind that to an external address carries a lot of assumptions with it that CarbonDB probably shouldn't be making.

    Dropping the op log for single-node is fine

Do that for the first draft.  If we have reason we want to add it later (expanding the API to support undo or audit) then that's great and we can do that.  But I don't want to pressure us to implement all that stuff on the first pass.

Update the implementation.md with these decisions, guidance, and context.


---

      @tlsft/service-error is written down as a proposal, not yet decided.

I think we can proceed with implementation knowing loosely that this is a goal.

    - remote access stays outside carbondb:

good.

    - execute: its mode argument is renamed to method and marked as internal.

ok.  not a concern or priority of mine.

    calls i made in the doc that you may want to change:
    - single-node is switched on with [sync] mode = "none".

great.

    - in single-node mode, these calls reject: post [@carbon]/members, post [@carbon]/join, post [@carbon]/sync/start, and post [device]/origin for a second device.
    - in single-node mode, get [@carbon]/status returns sync: 'off', peers: [], and full access for the local identity.

yes.  sensible and well defined.

    - i wrote watch(key, fn) to match story 9, not watch(key, input, fn).

fine.  no comment.

    not updated yet: stories/services.md and the readme's api sketch don't mention single-node mode or service errors.

readme might need this but i don't think services.md does.  it's orthoganal to the service listing.

ok, update planning documents.

---


    - **Names:** `system`, `identity:Brandon`...

Drop the talk about `@`.  Make sure all of the docs are consistent with this change (dropping @ from space names) and then stop talking about it (except when you are talking about local services).  Make the space names like `identity:brandon` actually compliant with the kebab case slug format that you mentioned in the same paragraph.

    - **`[space]`** runs in every replicated space, and identity spaces also run `[identity]` and `[device]`.

Separate bullet points for [identity] and [device].

    Creating a space cannot be undone. If a rewind rejects a parent's ref, the space stays, unattached.

This should be revisited in stories/backlog/security-review.md

    - Parenthood is set at creation. Reassigning it is undefined.

Review in a backlog story.

    - Adding someone to a child space does not add them to its parent.  But it may be prevented based on RBAC in the parent.

We need a backlog story to look into this further.

    - Other replicas tie a device to an identity through the p2panda group, without reading the identity log.

Revisit this in the backlog.  It sounds like a divergence from Keybase which we should more fully understand the consequences of.  Are we able to gain the semantics that we want purely based on this?  Particularly keybase would store public proofs of ownership of various social media accounts, or other proofs that bolster the trust and reputation of an identity... Perhaps "pull" access for the Identity chain should be permissable?  Or maybe we need some other implementation to accomodate this use case.

    - An op from a device not yet known to resolve to an identity waits unapplied until the group catches up.

What does this mean?  That's very vague and lacking context.  Why would it be "not yet known to resolve to an identity", and what "group" is catching up?

---

    "An op from a device not yet known to resolve to an identity waits…"  is probably redundant, because p2panda already handles it

I deleted it.

    - The per-identity key is an op in the identity space and needs no per-device encryption, because only the identity's devices can read it.

This is based on a presumption that could change.  Are you talking about an assymetric, or a symmetric encryption key?  I am familiar with the use case: we may want to encrypt data for use by the identity, and all devices within the identity should be able to decrypt and have access to that...

But it seems important to sort out: whether the identity log is intended to be private only, for synchronizing the devices, or whether it is intendend to be public.  I think that perhaps the right way to handle this is that you do not put any cryptographic secrets into replicated spaces: you store them on the devices and interact with them through [@carbon].  Is there a more fundamental signaling/messaging capability exposed by iroh or p2panda that can be used for this?  E.g. I know a peer's ID, I should be able to look this up and send a system message.  And this should be part of the handshake process of adding a new device to an identity.  As well as performing a key rotation.

Does what I am saying feel appropriate, or are we reinventing some portion of p2panda where this is already done for us in a better way?

---


    An identity is a p2panda group of its devices. Any space with that group as a member gets a symmetric group secret, which p2panda shares with every device

Ok, so group membership already implies that the device holds a symmetric key, and can therefore access the data.  Which means that we don't need to reinvent this by holding it in the Oplog itself: that serves no purpose.  Yes?

    Is the identity log private or public? Private: it keeps the identity's devices in sync. Public proofs stay in the backlog story

That's a bit bullheaded.  We just agreed to take out what I think was the only private portion of it.  The private portions all live on the local device, or managed internally by p2panda.  The public proofs need to live somewhere.  Do you have a better idea than the identity space itself?  Look at the API in stories/services.md.  Are there things that would be operating in the identity space that we want to or need to keep private?

---

    Of everything in the identity space, only the refs need to stay private

Can the refs be located in a different space?  What would that look like?  Perhaps this is what the "personal" space is for?  identity should remain pure, it has one focus, and "personal" is where the hierarchy for personal ownership begins?  Similar to the separation between / and /home on a Linux file system.

    Names and the manage tag are mildly sensitive

Can they be removed?

    The identity space is the right home for public proofs, as long as the refs move out. I don't have a better place for proofs.

I agree.  The shape of this is so close to what we want, I think it's desireable to give proper boundaries and gaurantees so that it can be used in this way.

    where do the refs go? The personal space is the obvious candidate

I see we came to the same conclusion.

    The tradeoff is that other identities may join a personal space

That's a good thing.  I have already mentioned scenarios like power of attorney or a caretaker.  I think it's very elegant that we can support this and not conflate identity with it.

    - Plain signed log: p2panda-core's signed append-only logs, synced on a topic without p2panda-spaces. That's closest to Keybase's sigchain.

That sounds acceptable to me.

    Grant Read to whoever asks:

Does not sound ideal.  Involves communication, presumes network connectivity.  There are cases where you want to publish known facts.

I think that:

1) the default case should be privacy.  Facilities should be available for manual, on demand serialization of the "identity" log.  Applications can decide out of band, outside of CarbonDB, if they want to post those somewhere, nostr, website, etc.
2) auto-publishing "identity" to anyone who subscribes can be turned on as a config.toml setting

What do you think about that?

---

    - POST [@carbon]/identity creates both spaces.

Should we do that?  Or separate the two.  I think the application could be in control of this.  Part of the onboarding lifecycle is to create an identity, and then create a personal space.  But maybe some applications don't care about, and don't want a personal space?

    Finding the personal space without a ref

Oh.  This is probably the answer.  Put a `ref` from "identity" to "personal".  It's not much of a revelation to say "this identity has a personal space, and you can't see it".  That preserves the reference without revealing much of importance.  And then company membership can also be nested inside "personal" rather than attaching it directly to "identity".  "Don't put anything in identity that you don't want published" then becomes a convention and a warning to the user, with nesting in another group as the pathway to privacy.

    Device names and tags can move to the personal space.
    - Names are only for display.
    - Tags are read only by the identity's own devices, to decide which of them get p2panda Manage in a space. p2panda enforces Manage per device. Other spaces check manage per identity, from their own logs. Nobody outside needs the tags.
    - [device]/origin in the identity space then carries only the public key.

Interesting.  If we make devices live in "personal" then it seems like more of a standard, always required space.  It makes sense in terms of privacy: just because I want to tell you about my identity doesn't mean I want to list out all of my personal devices.  You can probably determine that anyway if we're peers on the network... but we don't have to put it in an explicitly public record.

    A manual export goes stale

Yes.  But people who choose manual mode are willing to accept the inconvenience for privacy.

    "Private" doesn't hide device keys from people you share a space with

Right.  But we don't need to duplicate it.

I think the main question here is whether "private" should be canonical, always present, or whether it should be situational and user/app discretion.

---

    A proof such as "I own this GitHub account" is signed by a device. An outsider verifies it by tracing that device back to the identity's origin through [device]/origin entries

Ah.  That's the connection.

    - Identity: device public keys only. This is the authorization chain, and it reveals how many devices the identity has.
    - Personal: device names and tags, which form the inventory you'd rather not publish.

Ok.

Proceed in updating the plans.

---

    - Personal space services are named in createIdentity({…, services}). A space's services are fixed at creation

Do we have anything in the backlog to describe adding services after point of space creation?  Let's add it if not.  I think it can be done in a straightforward manner: just another Op, and anything after that point can call the additional services.

- [device] runs in both spaces:
  - In identity, POST [device]/origin {pubkey}.
  - In personal, PUT [device]/devices/:pubkey and GET [device]/devices.

Eh... I don't really like this.  What do you think about just encrypting the fields you want to keep private rather than spreading out the API calls like this?

---

    Proposal: one call, with private fields hidden from exports instead of encrypted.

That sounds really complicated: a significant extension in the API for oplog/service calls that introduces meta data on fields, and an interpretation of what private means.  That creates unnecessary overhead for anyone trying to understand our fundamental abstractions of service keys and calls.

But I also don't like spreading [device] out over two spaces, with some endpoints viable in one space, and some viable in the other.  That's confusing.  And the paths themselves start to get weird because you're having to duplicate functionality and invent some new path for it that is attempting to still be sensible.

I want to point out:

1) CarbonDB in general does not make any assumptions about the payload of service calls: they are arbitrary binary data
2) services implement schemas, which CarbonDB is unaware of.  So CarbonDB does not know anything about zodly tags on fields such as what MicroQL uses.
3) CarbonDB provides several "built-in" services that accomplish internal implementation details
4) Nothing constrains the shape of its built-in services, or how they operate, or whether they operate different from user provided services

Therefore, we don't need to create an abstraction that works for all possible services.  We could allow this to be unique.  However, I think what IS currently defined is that a CALL goes out to the Oplog and is replicated first, before it reaches any service handlers.  So the service handler itself doesn't have the ability to interpret or modify anything before it hits the oplog.

    The export drops the plaintext and keeps the hash, so the signature still verifies and the name can't be guessed. This is how selective disclosure works in SD-JWT.

I don't understand what you're talking about.

    - [device] is back to one space, so the personal space no longer needs it and holds only refs and personal data.

What if you just put the fields that we want to keep private inside the `ref` to the device?  What if `ref` supports a `meta` field with arbitrary nesting?  Then we don't need to add any new API methods, and we don't need to do any tricks for encrypted fields.

---

Can you go through "Left for later" in implementation.md one by one, and we either decide to put them in the backlog, incorporate them into the implementation, or drop them? (backlog, impl, drop)

---

I earlier defined listen syntax as:

    carbon.call('POST [@carbon]/listen', {
        to: 'POST [space]/members',
        in: ['POST [logger]/log'],
        out: ['POST [logger]/log']
        })

I updated this with @carbon and [space] per our other agreed upon changes, but is otherwise unmodified from my original proposal.

Arguably, this should be called "trigger" not "listen".

    watch is something else

Does watch exist?  This is another horse that we've beaten to death, I feel like you're referring to outdated information that simply didn't get updated.

Does any of this remain unclear?

---

    My proposal: POST [space]/trigger {to, in, out},

Make it so.

    As far as I can find, watch still exists and nothing dropped it. It's in story 9, the app interface

Ok.  The use case for this was powering UIs, setting up binding?

    says a listener "writes only SQLite"

This is something YOU proposed, and which I have struck down several times now.  Please knock it off.

    Your design says a trigger has no handler and only forwards to other services

Yes.

---

I'm now satisfied that I've gone through the parts of implementation.md that I was most interested in.  What I did not go through:

## The op
## Applying an op
### Reads
## Late ops and rewind
## Publishing
## Repeatability
## Rules for service authors
## Members, access and manage
## Invitations
## Removing a member
## Sync, status and partitions
## Core and runtime
### Handles
## Porting Otto later

These are things that I think we've either A) discussed to death and agree upon, or B) do not need to be clarified at this time, because we have enough clarity to begin our implementation.  Do you agree?  Is there anything you'd like to raise to my attention before I have you begin implementation?

---

    1. Is the CarbonDB repo public?

Yes CarbonDB is and will remain public.  Otto is not, but MicroQL is.  The Otto references... might confuse people a bit, but I think eventually I will open source Otto, and it would be too confusing to try and have a conversation about it without naming it.

However I think you're right to move Otto references into perspectives/brandon/.  That's a good place for them to live.

    2. The riskiest part of build step 1 is untested

Acknowledged.  I have faith we'll figure something out there, and I'm not super attached to the exact form.

    - @tlsft/service-error starts as a package in this repo

Put it in Otto, along side @tlsft/service-key

    For the hello world, main() registers services with carbon.catalog.add(module),

Make that carbon.services.add('service-name', impl).  We're in JS land when we call this, so impl is simply the exported service object.

Update the plans and I'll commit the repo.

---

    Four lines in the docs still put validation in the runtime, so I'd fix them:

Yes, get rid of that.

Then proceed with implementation.  Ideally accomplish the entire agenda, and only stop if you have something truly show stopping.  Feel free to make decisions and just note them in the agenda if you want my review.  You have a lot of detailed context from all the planning we have done in the last few days, so don't stop for minor issues.

Thanks and godspeed!
