import {describe, it, beforeEach} from 'node:test'
import assert from 'node:assert'
import {mkdtempSync, writeFileSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {spawn} from 'node:child_process'
import {once} from 'node:events'
import {dirname, join} from 'node:path'
import {conflict} from '@tlsft/service-error'
import {configure} from '../src/index.js'
import {applyMigrations} from '../src/migrations.js'

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const tick = () => sleep(2)

// Where a test service's service.toml is: a package of its own under services/,
// though its calls are written here.
const manifestOf = (name) => new URL(`./services/${name}/service.toml`, import.meta.url)

// [notes] must settle each call within the timeout its service.toml names.
const TIMEOUT = 100

// A GET that reads at once but answers only when the test opens the gate, so a
// commit can land while it is in flight.
let gate = Promise.resolve()
function closeGate() {
  let open
  gate = new Promise((resolve) => { open = resolve })
  return open
}

// A small service exercising what the runtime offers a handler.
const notes = {
  manifest: manifestOf('notes'),
  migrations: [
    {name: '0001_note', sql: 'CREATE TABLE note (id TEXT PRIMARY KEY, author TEXT NOT NULL, text TEXT NOT NULL, at INTEGER NOT NULL) WITHOUT ROWID'},
    {name: '0002_tag', sql: 'CREATE TABLE tag (name TEXT PRIMARY KEY) WITHOUT ROWID'}
  ],
  init: (db) => applyMigrations(db, notes.migrations),
  calls: {
    'POST [notes]/notes': {
      handler: async ({input, op, env: {db}}) => {
        const id = op.id()
        db.run('INSERT INTO note (id, author, text, at) VALUES (?, ?, ?, ?)', [id, op.author, input.text, op.time])
        await tick()
        if (input.refuse) throw conflict('Refused after writing', {id})
        db.run('INSERT INTO note (id, author, text, at) VALUES (?, ?, ?, ?)', [op.id(), op.author, `${input.text} again`, op.time])
        return {id, author: op.author, device: op.device}
      }
    },
    // Writes, then outlives its timeout before writing again.
    'POST [notes]/late': {
      handler: async ({input, op, env: {db}}) => {
        db.run('INSERT INTO note (id, author, text, at) VALUES (?, ?, ?, ?)', [op.id(), op.author, input.text, op.time])
        await sleep(TIMEOUT * 1.5)
        db.run('INSERT INTO note (id, author, text, at) VALUES (?, ?, ?, ?)', [op.id(), op.author, `${input.text} again`, op.time])
      }
    },
    'POST [notes]/duplicates': {
      handler: ({env: {db}}) => {
        db.run("INSERT INTO note (id, author, text, at) VALUES ('same', 'a', 'x', 0)")
        db.run("INSERT INTO note (id, author, text, at) VALUES ('same', 'a', 'x', 0)")
      }
    },
    // Writes a note, reads through a call inside the op, then optionally makes
    // another call and refuses: the shapes a nested transaction must get right.
    'POST [notes]/nested': {
      handler: async ({input, op, env: {db}, call}) => {
        db.run('INSERT INTO note (id, author, text, at) VALUES (?, ?, ?, ?)', [op.id(), op.author, input.text, op.time])
        const seen = (await call('GET [notes]/notes')).map((row) => row.text)
        if (input.inner) await call('POST [notes]/notes', input.inner).catch((error) => { if (!input.catch) throw error })
        if (input.refuse) throw conflict('Refused after its calls')
        return {seen}
      }
    },
    'POST [notes]/unawaited': {
      handler: ({call}) => {
        call('POST [notes]/notes', {text: 'late'}).catch(() => {})
      }
    },
    'GET [notes]/notes': {handler: ({env: {db}}) => db.all('SELECT text FROM note ORDER BY at, text')},
    'GET [notes]/writes': {handler: ({call}) => call('POST [notes]/tags', {name: 'red'})},
    'POST [notes]/tags': {handler: ({input, env: {db}}) => db.run('INSERT INTO tag (name) VALUES (?)', [input.name])},
    'GET [notes]/broken': {handler: ({env: {db}}) => db.all('SELECT * FROM nowhere')},
    'GET [notes]/gated': {
      handler: async ({env: {db}}) => {
        const notes = db.all('SELECT text FROM note ORDER BY at, text')
        await gate
        return notes
      }
    },
    // Reads the notes only once a tag exists, so its read set grows.
    'GET [notes]/tagged': {
      handler: async ({env: {db}}) => {
        const tags = db.all('SELECT name FROM tag')
        const notes = tags.length ? db.all('SELECT text FROM note ORDER BY at, text') : []
        await gate
        return {tags: tags.map((tag) => tag.name), notes: notes.map((note) => note.text)}
      }
    }
  }
}

function configFile(text = 'system = "data/system.db"\n[sync]\nmode = "none"\n') {
  const directory = mkdtempSync(join(tmpdir(), 'carbondb-'))
  const path = join(directory, 'carbondb.toml')
  writeFileSync(path, text)
  return path
}

async function start(path) {
  const carbon = await configure(path)
  carbon.services.add('notes', notes)
  return carbon
}

// A space made in `parent`, as an app makes one: a call there, then a ref to it.
async function spaceIn(parent, input) {
  const {kind, slug} = await parent.call('POST [@carbon]/space', input)
  return parent.ref(`${kind}:${slug}`)
}

let config, carbon, alice, identity, personal
beforeEach(async () => {
  config = configFile()
  carbon = await start(config)
  alice = await carbon.call('POST [@carbon]/identity', {slug: 'alice', device: 'Pixel9', services: ['notes']})
  identity = carbon.ref('identity:alice')
  personal = identity.ref('personal:alice')
})

const texts = async (space) => (await space.call('GET [notes]/notes')).map((row) => row.text)

describe('configure', () => {
  it('refuses replicated mode, which is not built yet', async () => {
    await assert.rejects(configure(configFile('system = "data/system.db"\n')), {status: 501, code: 'NOT_BUILT'})
  })
})

describe('identities', () => {
  it('lists the identity and resumes it in a fresh start with the same id and device', async () => {
    const [listed] = await carbon.call('GET [@carbon]/identities')
    assert.deepStrictEqual([listed.slug, listed.id, listed.device], ['alice', alice.id, alice.device])
    const again = (await start(config)).ref('identity:alice')
    assert.deepStrictEqual(again.identity, alice)
  })

  it('keeps the device key in the identity space and its name and tags in the personal space', async () => {
    assert.deepStrictEqual(await identity.call('GET [device]/devices'), [alice.device])
    assert.deepStrictEqual(await personal.call('GET [device]/refs'), [{pubkey: alice.device, name: 'Pixel9', tags: ['manage']}])
  })

  it('reaches the personal space through refs from the system handle', () => {
    assert.strictEqual(carbon.ref('identity:alice').ref('personal:alice').space, personal.space)
  })

  it('refuses a ref that does not exist', () => {
    assert.throws(() => carbon.ref('identity:sally'), {status: 404})
    assert.throws(() => personal.ref('space:nowhere'), {status: 404})
    assert.throws(() => personal.ref('nowhere'), TypeError)
  })
})

describe('services', () => {
  it('refuses registering a service twice, or under a CarbonDB service name', () => {
    assert.throws(() => carbon.services.add('notes', notes), {status: 409})
    assert.throws(() => carbon.services.add('space', notes), {status: 409})
  })

  it('hands a handler its binding in env, under the name its service.toml gives it', async () => {
    carbon.services.add('stash', {
      manifest: manifestOf('stash'),
      init: (db) => db.run('CREATE TABLE IF NOT EXISTS item (id TEXT PRIMARY KEY, name TEXT NOT NULL) WITHOUT ROWID'),
      calls: {
        'POST [stash]/items': {handler: ({input, op, env}) => env.store.run('INSERT INTO item (id, name) VALUES (?, ?)', [op.id(), input.name])},
        'GET [stash]/items': {handler: ({env}) => ({bindings: Object.keys(env), items: env.store.all('SELECT name FROM item')})}
      }
    })
    const stash = await spaceIn(personal, {name: 'Stash', services: ['stash']})
    await stash.call('POST [stash]/items', {name: 'kept'})
    assert.deepStrictEqual(await stash.call('GET [stash]/items'), {bindings: ['store'], items: [{name: 'kept'}]})
  })

  it('refuses a service without a service.toml it can read', () => {
    assert.throws(() => carbon.services.add('bare', {calls: {}}), {status: 400, message: /exports no manifest/})
    assert.throws(() => carbon.services.add('lost', {manifest: manifestOf('lost'), calls: {}}), {status: 400, message: /Cannot read/})
  })

  it('refuses an identity or space naming a service this device has not registered', async () => {
    await assert.rejects(carbon.call('POST [@carbon]/identity', {slug: 'sally', device: 'x', services: ['crm']}), {status: 400, details: {services: ['crm']}})
    await assert.rejects(personal.call('POST [@carbon]/space', {name: 'Sales', services: ['crm']}), {status: 400})
  })

  it('keeps data across a restart that registers the service again, and refuses the space when it does not', async () => {
    await personal.call('POST [notes]/notes', {text: 'kept'})
    const restarted = (await start(config)).ref('identity:alice').ref('personal:alice')
    assert.deepStrictEqual(await texts(restarted), ['kept', 'kept again'])
    const unregistered = (await configure(config)).ref('identity:alice').ref('personal:alice')
    await assert.rejects(unregistered.call('GET [notes]/notes'), {status: 404, details: {service: 'notes'}})
    assert.deepStrictEqual(unregistered.runState(), {state: 'initializing', services: ['notes']})
  })
})

describe('calls', () => {
  it('commits an async handler, with the author and device from the acting identity', async () => {
    const result = await personal.call('POST [notes]/notes', {text: 'hello'})
    assert.deepStrictEqual([result.author, result.device], [alice.id, alice.device])
    assert.deepStrictEqual(await texts(personal), ['hello', 'hello again'])
  })

  it('rolls back a rejected handler, and the caller gets its service error intact', async () => {
    await assert.rejects(personal.call('POST [notes]/notes', {text: 'nope', refuse: true}), (error) => {
      assert.deepStrictEqual([error.status, error.code, error.message], [409, 'CONFLICT', 'Refused after writing'])
      assert.match(error.details.id, /^[0-9a-f]{32}$/)
      return true
    })
    assert.deepStrictEqual(await texts(personal), [])
  })

  it('rolls back a failing statement and reports it as a 500', async () => {
    await assert.rejects(personal.call('POST [notes]/duplicates'), {status: 500, message: /UNIQUE constraint failed/})
    assert.deepStrictEqual(await texts(personal), [])
  })

  it('applies concurrent calls one at a time', async () => {
    await Promise.all(['a', 'b', 'c'].map((text) => personal.call('POST [notes]/notes', {text})))
    assert.deepStrictEqual(await texts(personal), ['a', 'a again', 'b', 'b again', 'c', 'c again'])
  })

  it('reads committed state while a write is still running', async () => {
    const writing = personal.call('POST [notes]/notes', {text: 'slow'})
    assert.deepStrictEqual(await texts(personal), [])
    assert.deepStrictEqual(await personal.call('GET [space]/refs'), [])
    await writing
    assert.deepStrictEqual(await texts(personal), ['slow', 'slow again'])
  })

  const nested = [
    {why: 'a call inside a handler sees its writes, across awaits', input: {text: 'outer'}, seen: ['outer'], committed: ['outer']},
    {why: 'a call that succeeds commits with the op', input: {text: 'outer', inner: {text: 'in'}}, seen: ['outer'], committed: ['in', 'in again', 'outer']},
    {why: 'a failing call the handler catches rolls back only its own writes', input: {text: 'outer', inner: {text: 'in', refuse: true}, catch: true}, seen: ['outer'], committed: ['outer']},
    {why: 'a failing call the handler does not catch rolls back the whole op', input: {text: 'outer', inner: {text: 'in', refuse: true}}, status: 409, committed: []},
    {why: 'a handler failing after its calls rolls them back too', input: {text: 'outer', inner: {text: 'in'}, refuse: true}, status: 409, committed: []}
  ]

  for (const {why, input, seen, status, committed} of nested) {
    it(why, async () => {
      const call = personal.call('POST [notes]/nested', input)
      if (status) await assert.rejects(call, {status})
      else assert.deepStrictEqual((await call).seen, seen)
      assert.deepStrictEqual(await texts(personal), committed)
    })
  }

  it('fails a handler that finishes before the calls it made, and commits none of them', async () => {
    await assert.rejects(personal.call('POST [notes]/unawaited'), {status: 500, message: /finished before the calls it made/})
    await tick()
    assert.deepStrictEqual(await texts(personal), [])
  })

  const refused = [
    {key: 'GET [notes]/writes', status: 400, why: 'a GET calling a write'},
    {key: 'GET [chat]/messages', status: 404, why: 'a service the space does not run'},
    {key: 'GET [notes]/nothing', status: 404, why: 'a resource the service does not have'},
    {key: 'DELETE [notes]/notes', status: 405, why: 'a method the resource does not answer'},
    {key: 'not a key', status: 400, why: 'something that is not a service key'},
    {key: 'GET [@elsewhere]/x', status: 404, why: 'a local service this device does not run'},
    {key: 'POST [@carbon]/members', status: 501, why: 'a call that needs peers'}
  ]

  for (const {key, status, why} of refused) {
    it(`refuses ${why}`, async () => {
      await assert.rejects(personal.call(key, {}), {status})
    })
  }
})

describe('spaces', () => {
  it('creates a space nested in the calling one, with its own slug and status', async () => {
    const room = await spaceIn(personal, {name: "Randall's Playhouse", services: ['notes']})
    const [listed] = (await personal.call('GET [space]/refs')).filter((ref) => ref.kind === 'space')
    assert.deepStrictEqual([listed.slug, listed.space], ['randalls-playhouse', room.space])
    assert.strictEqual(personal.ref('space:randalls-playhouse').space, room.space)

    const status = await room.call('GET [@carbon]/status')
    assert.deepStrictEqual(
      {name: status.name, services: status.services, manage: status.manage, access: status.access, sync: status.sync, peers: status.peers},
      {name: "Randall's Playhouse", services: ['notes'], manage: [alice.id], access: ['manage'], sync: 'off', peers: []}
    )
    await room.call('POST [notes]/notes', {text: 'in the room'})
    assert.deepStrictEqual(await texts(room), ['in the room', 'in the room again'])
    assert.deepStrictEqual(await texts(personal), [])
  })
})

describe('watch', () => {
  // The results a watch delivers, and a way to wait for the next one.
  function watching(space, key) {
    const results = []
    let next
    const stop = space.watch(key, (value, error) => {
      results.push(error ? {error: error.status} : value)
      next?.()
    })
    const arrival = () => new Promise((resolve) => { next = resolve })
    return {results, stop, arrival}
  }

  it('runs the GET, then again after a commit that writes a table it read', async () => {
    const {results, arrival} = watching(personal, 'GET [notes]/notes')
    await arrival()
    personal.call('POST [notes]/notes', {text: 'hi'})
    await arrival()
    assert.deepStrictEqual(results, [[], [{text: 'hi'}, {text: 'hi again'}]])
  })

  it('does not re-run after a commit to a table it did not read', async () => {
    const {results, arrival} = watching(personal, 'GET [notes]/notes')
    await arrival()
    await personal.call('POST [notes]/tags', {name: 'red'})
    await new Promise((resolve) => setImmediate(resolve))
    await tick()
    assert.deepStrictEqual(results, [[]])
  })

  it('stops when asked', async () => {
    const {results, stop, arrival} = watching(personal, 'GET [notes]/notes')
    await arrival()
    stop()
    await personal.call('POST [notes]/notes', {text: 'unseen'})
    await new Promise((resolve) => setImmediate(resolve))
    await tick()
    assert.deepStrictEqual(results, [[]])
  })

  it('watches a builtin GET too', async () => {
    const {results, arrival} = watching(personal, 'GET [space]/refs')
    await arrival()
    personal.call('POST [@carbon]/space', {name: 'Lobby', services: ['notes']})
    await arrival()
    assert.deepStrictEqual(results.map((refs) => refs.map((ref) => ref.slug)), [[], ['lobby']])
  })

  it('has run again by the time a call whose commit touched it resolves', async () => {
    const {results, arrival} = watching(personal, 'GET [notes]/notes')
    await arrival()
    await personal.call('POST [notes]/notes', {text: 'seen'})
    assert.deepStrictEqual(results.at(-1), [{text: 'seen'}, {text: 'seen again'}])
  })

  it('has run again by the time a call resolves when watched and called at once', async () => {
    const {results} = watching(personal, 'GET [notes]/notes')
    await personal.call('POST [notes]/notes', {text: 'at once'})
    assert.deepStrictEqual(results.at(-1), await personal.call('GET [notes]/notes'))
    assert.deepStrictEqual(results.at(-1), [{text: 'at once'}, {text: 'at once again'}])
  })

  it('runs again after a commit lands while its GET is in flight', async () => {
    const open = closeGate()
    const {results} = watching(personal, 'GET [notes]/gated')
    const posted = personal.call('POST [notes]/notes', {text: 'during'})
    await tick()
    await tick()
    open()
    await posted
    assert.deepStrictEqual(results, [[], [{text: 'during'}, {text: 'during again'}]])
    assert.deepStrictEqual(results.at(-1), await personal.call('GET [notes]/gated'))
  })

  it('judges a commit in flight by what the run read, even when its read set grew', async () => {
    const {results, arrival} = watching(personal, 'GET [notes]/tagged')
    await arrival()
    const open = closeGate()
    const tagged = personal.call('POST [notes]/tags', {name: 'red'})
    const posted = personal.call('POST [notes]/notes', {text: 'seen'})
    await tick()
    await tick()
    open()
    await Promise.all([tagged, posted])
    assert.deepStrictEqual(results.at(-1), {tags: ['red'], notes: ['seen', 'seen again']})
    assert.deepStrictEqual(results.at(-1), await personal.call('GET [notes]/tagged'))
  })

  it('answers a GET at once while a watch is in flight', async () => {
    const open = closeGate()
    watching(personal, 'GET [notes]/gated')
    const read = await Promise.race([personal.call('GET [notes]/notes'), tick().then(() => 'held')])
    open()
    assert.deepStrictEqual(read, [])
  })

  it('hands a failing GET its error', async () => {
    const {results, arrival} = watching(personal, 'GET [notes]/broken')
    await arrival()
    assert.deepStrictEqual(results, [{error: 500}])
  })
})

describe('timeout', () => {
  const timeouts = () => process.getActiveResourcesInfo().filter((resource) => resource === 'Timeout').length

  it('clears the deadline of a call that settled', async () => {
    const before = timeouts()
    await personal.call('POST [notes]/notes', {text: 'quick'})
    await personal.call('GET [notes]/notes')
    assert.ok(timeouts() <= before, 'a settled call leaves no timeout behind')
  })

  it('settles a call its handler has not settled in time, rolling back its writes and dropping its late answer', async () => {
    await assert.rejects(personal.call('POST [notes]/late', {text: 'late'}), {status: 504, code: 'TIMED_OUT'})
    assert.deepStrictEqual(await texts(personal), [])
    await sleep(TIMEOUT)
    await personal.call('POST [notes]/notes', {text: 'next'})
    assert.deepStrictEqual(await texts(personal), ['next', 'next again'])
  })
})

describe('run state', () => {
  // Hold a space's file from another process, as a second copy of the app
  // would; resolves with a function that lets go of it.
  async function holdLock(space) {
    const path = join(dirname(config), 'data', `${space.space}.db`)
    const holding = `const db = new (require('node:sqlite').DatabaseSync)(${JSON.stringify(path)}); db.exec('BEGIN IMMEDIATE'); console.log('held'); process.stdin.on('end', () => db.exec('ROLLBACK')); process.stdin.resume()`
    const holder = spawn(process.execPath, ['--no-warnings', '-e', holding], {stdio: ['pipe', 'pipe', 'inherit']})
    await once(holder.stdout, 'data')
    return async () => {
      holder.stdin.end()
      await once(holder, 'exit')
    }
  }

  it('stops a space on a system failure, failing its calls and watches with a 503, until it is resumed', async () => {
    const states = []
    personal.watchRunState((state) => states.push(state.state))
    const results = []
    let next
    personal.watch('GET [notes]/notes', (value, error) => {
      results.push(error ? {error: error.code} : value)
      next?.()
    })
    const arrival = () => new Promise((resolve) => { next = resolve })
    await arrival()
    assert.deepStrictEqual(personal.runState(), {state: 'idle'})

    const release = await holdLock(personal)
    const stopped = arrival()
    await assert.rejects(personal.call('POST [notes]/notes', {text: 'lost'}), {status: 503, code: 'SPACE_STOPPED'})
    await stopped
    assert.deepStrictEqual(results.at(-1), {error: 'SPACE_STOPPED'})
    const {state, error} = personal.runState()
    assert.deepStrictEqual([state, error.code], ['stopped', 'SYSTEM_FAILURE'])
    await assert.rejects(personal.call('GET [notes]/notes'), {status: 503})

    await release()
    const resumed = arrival()
    personal.resume()
    await resumed
    assert.deepStrictEqual(results.at(-1), [])
    await personal.call('POST [notes]/notes', {text: 'kept'})
    assert.deepStrictEqual(await texts(personal), ['kept', 'kept again'])
    assert.deepStrictEqual(states, ['stopped', 'idle', 'applying', 'idle'])
  })
})

describe('refs', () => {
  it('reach a space at once, also after a restart', async () => {
    const {space} = await personal.call('POST [@carbon]/space', {name: 'Lobby', services: ['notes']})
    assert.strictEqual(personal.ref('space:lobby').space, space)
    const restarted = await start(config)
    assert.strictEqual(restarted.ref('identity:alice').ref('personal:alice').ref('space:lobby').space, space)
  })
})

describe('schema', () => {
  it("is set up by a service's init, which applies only the migrations not applied yet", async () => {
    await personal.call('POST [notes]/tags', {name: 'red'})
    const grown = {
      ...notes,
      migrations: [...notes.migrations, {name: '0003_color', sql: "ALTER TABLE tag ADD COLUMN color TEXT NOT NULL DEFAULT 'none'"}],
      init: (db) => applyMigrations(db, grown.migrations),
      calls: {...notes.calls, 'GET [notes]/tags': {handler: ({env: {db}}) => db.all('SELECT name, color FROM tag')}}
    }
    const restarted = await configure(config)
    restarted.services.add('notes', grown)
    assert.deepStrictEqual(await restarted.ref('identity:alice').ref('personal:alice').call('GET [notes]/tags'), [{name: 'red', color: 'none'}])
  })

  it('refuses a service whose table takes its keys from SQLite, and says how to fix it', async () => {
    carbon.services.add('counter', {manifest: manifestOf('counter'), init: (db) => db.run('CREATE TABLE IF NOT EXISTS hit (id INTEGER PRIMARY KEY AUTOINCREMENT)'), calls: {}})
    await assert.rejects(personal.call('POST [@carbon]/space', {name: 'Counted', services: ['counter']}), (error) => {
      assert.deepStrictEqual([error.status, error.details], [400, {table: 'hit'}])
      assert.match(error.message, /must come from the op that creates it.*op_id\(\)/)
      return true
    })
  })

  it("keeps a service inside its own tables, even when it names another service's", async () => {
    carbon.services.add('snoop', {manifest: manifestOf('snoop'), calls: {'GET [snoop]/refs': {handler: ({env: {db}}) => db.all('SELECT * FROM "space/db/ref"')}}})
    const room = await spaceIn(personal, {name: 'Snooped', services: ['snoop']})
    await assert.rejects(room.call('GET [snoop]/refs'), {status: 500, message: /no such table: snoop\/db\/space\/db\/ref/})
  })
})
