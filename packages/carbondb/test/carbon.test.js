import {describe, it, beforeEach} from 'node:test'
import assert from 'node:assert'
import {mkdtempSync, writeFileSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {conflict} from '@tlsft/service-error'
import {configure} from '../src/index.js'

const tick = () => new Promise((resolve) => setTimeout(resolve, 2))

// A small service exercising what the runtime offers a handler.
const notes = {
  migrations: [
    'CREATE TABLE note (id TEXT PRIMARY KEY, author TEXT NOT NULL, text TEXT NOT NULL, at INTEGER NOT NULL)',
    'CREATE TABLE tag (name TEXT PRIMARY KEY)'
  ],
  calls: {
    'POST [notes]/notes': {
      handler: async ({input, op, db}) => {
        const id = op.id()
        db.run('INSERT INTO note (id, author, text, at) VALUES (?, ?, ?, ?)', [id, op.author, input.text, op.time])
        await tick()
        if (input.refuse) throw conflict('Refused after writing', {id})
        db.run('INSERT INTO note (id, author, text, at) VALUES (?, ?, ?, ?)', [op.id(), op.author, `${input.text} again`, op.time])
        return {id, author: op.author, device: op.device}
      }
    },
    'POST [notes]/duplicates': {
      handler: ({db}) => {
        db.run("INSERT INTO note (id, author, text, at) VALUES ('same', 'a', 'x', 0)")
        db.run("INSERT INTO note (id, author, text, at) VALUES ('same', 'a', 'x', 0)")
      }
    },
    // Writes a note, reads through a call inside the op, then optionally makes
    // another call and refuses: the shapes a nested transaction must get right.
    'POST [notes]/nested': {
      handler: async ({input, op, db, call}) => {
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
    'GET [notes]/notes': {handler: ({db}) => db.all('SELECT text FROM note ORDER BY at, text')},
    'GET [notes]/writes': {handler: ({call}) => call('POST [notes]/tags', {name: 'red'})},
    'POST [notes]/tags': {handler: ({input, db}) => db.run('INSERT INTO tag (name) VALUES (?)', [input.name])},
    'GET [notes]/broken': {handler: ({db}) => db.all('SELECT * FROM nowhere')}
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

let config, carbon, identity, personal
beforeEach(async () => {
  config = configFile()
  carbon = await start(config)
  ;({identity, personal} = await carbon.createIdentity({slug: 'alice', device: 'Pixel9', services: ['notes']}))
})

const texts = async (space) => (await space.call('GET [notes]/notes')).map((row) => row.text)

describe('configure', () => {
  it('refuses replicated mode, which is not built yet', async () => {
    await assert.rejects(configure(configFile('system = "data/system.db"\n')), {status: 501, code: 'NOT_BUILT'})
  })
})

describe('identities', () => {
  it('lists the identity and resumes it in a fresh start with the same id and device', async () => {
    const [listed] = await carbon.listIdentities()
    assert.deepStrictEqual([listed.slug, listed.id, listed.device], ['alice', identity.identity.id, identity.identity.device])
    const again = (await start(config)).resumeIdentity('alice')
    assert.deepStrictEqual(again.identity, identity.identity)
  })

  it('keeps the device key in the identity space and its name and tags in the personal space', async () => {
    assert.deepStrictEqual(await identity.call('GET [device]/devices'), [identity.identity.device])
    assert.deepStrictEqual(await personal.call('GET [device]/refs'), [{pubkey: identity.identity.device, name: 'Pixel9', tags: ['manage']}])
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

  it('refuses an identity or space naming a service this device has not registered', async () => {
    await assert.rejects(carbon.createIdentity({slug: 'sally', device: 'x', services: ['crm']}), {status: 400, details: {services: ['crm']}})
    await assert.rejects(personal.createSpace({name: 'Sales', services: ['crm']}), {status: 400})
  })

  it('keeps data across a restart that registers the service again, and refuses the space when it does not', async () => {
    await personal.call('POST [notes]/notes', {text: 'kept'})
    const restarted = (await start(config)).resumeIdentity('alice').ref('personal:alice')
    assert.deepStrictEqual(await texts(restarted), ['kept', 'kept again'])
    const unregistered = (await configure(config)).resumeIdentity('alice').ref('personal:alice')
    await assert.rejects(unregistered.call('GET [notes]/notes'), {status: 404, details: {service: 'notes'}})
  })
})

describe('calls', () => {
  it('commits an async handler, with the author and device from the acting identity', async () => {
    const result = await personal.call('POST [notes]/notes', {text: 'hello'})
    assert.deepStrictEqual([result.author, result.device], [identity.identity.id, identity.identity.device])
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
    const room = await personal.createSpace({name: "Randall's Playhouse", services: ['notes']})
    const [listed] = (await personal.listSpaces()).filter((ref) => ref.kind === 'space')
    assert.deepStrictEqual([listed.slug, listed.space], ['randalls-playhouse', room.space])
    assert.strictEqual(personal.ref('space:randalls-playhouse').space, room.space)

    const status = await room.status()
    assert.deepStrictEqual(
      {name: status.name, services: status.services, manage: status.manage, access: status.access, sync: status.sync, peers: status.peers},
      {name: "Randall's Playhouse", services: ['notes'], manage: [identity.identity.id], access: ['manage'], sync: 'off', peers: []}
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
    personal.createSpace({name: 'Lobby', services: ['notes']})
    await arrival()
    assert.deepStrictEqual(results.map((refs) => refs.map((ref) => ref.slug)), [[], ['lobby']])
  })

  it('hands a failing GET its error', async () => {
    const {results, arrival} = watching(personal, 'GET [notes]/broken')
    await arrival()
    assert.deepStrictEqual(results, [{error: 500}])
  })
})
