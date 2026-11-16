import {describe, it, before, after, beforeEach} from 'node:test'
import assert from 'node:assert'
import {mkdtempSync, readFileSync, writeFileSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {spawn} from 'node:child_process'
import {createServer} from 'node:http'
import {createServer as createTcpServer} from 'node:net'
import {once} from 'node:events'
import {dirname, join} from 'node:path'
import {configure} from '../src/index.js'
import {packageComponent} from '../src/native.js'

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

// A test service's package: the component `pretest` built from its source
// under fixtures/, with its service.toml.
const component = (name) => new URL(`../fixtures/${name}/${name}.wasm`, import.meta.url)

// A package's bytes with a custom section of its own appended, so its version
// is new and this process compiles it afresh.
const afresh = (bytes) => {
  const name = new TextEncoder().encode(`carbondb:test:${Date.now()}:${Math.random()}`)
  return Uint8Array.from([...bytes, 0, name.length + 1, name.length, ...name])
}

function configFile(text = 'system = "data/system.db"\n[sync]\nmode = "none"\n') {
  const directory = mkdtempSync(join(tmpdir(), 'carbondb-'))
  const path = join(directory, 'carbondb.toml')
  writeFileSync(path, text)
  return path
}

async function start(path) {
  const carbon = await configure(path)
  await carbon.services.add('notes', component('notes'))
  return carbon
}

// A channel made in `parent`, as an app makes one: a call there, a ref to it,
// then each of its services named in it.
async function channelIn(parent, {name, services = []}) {
  const {kind, slug} = await parent.call('POST [channel]/channel', {name})
  const channel = parent.ref(`${kind}:${slug}`)
  for (const service of services) await channel.call('POST [channel]/services', {name: service})
  return channel
}

// The id of the service a channel names `[name]`.
const serviceOf = async (channel, name) => (await channel.call('GET [channel]/services')).find((service) => service.name === name).service

let config, carbon, alice, identity, personal
beforeEach(async () => {
  config = configFile()
  carbon = await start(config)
  alice = await carbon.call('POST [@carbon]/identity', {slug: 'alice', device: 'Pixel9', services: ['notes']})
  identity = carbon.ref('identity:alice')
  personal = identity.ref('personal:alice')
})

const texts = async (channel) => (await channel.call('GET [notes]/notes')).map((row) => row.text)

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

  it("keeps the identity's devices, with their names and tags, in its own channel", async () => {
    assert.deepStrictEqual(await identity.call('GET [identity]/devices'), [{pubkey: alice.device, name: 'Pixel9', tags: ['manage']}])
    await assert.rejects(identity.call('POST [channel]/ref', {channel: 'c1', kind: 'team', name: 'Sales'}), {status: 403, message: /sealed/})
  })

  it('reaches the personal channel through refs from the system handle', () => {
    assert.strictEqual(carbon.ref('identity:alice').ref('personal:alice').channel, personal.channel)
  })

  it('refuses a ref that does not exist', () => {
    assert.throws(() => carbon.ref('identity:sally'), {status: 404})
    assert.throws(() => personal.ref('channel:nowhere'), {status: 404})
    assert.throws(() => personal.ref('nowhere'), TypeError)
  })
})

describe('services', () => {
  it('refuses registering a service twice, or under a CarbonDB service name', async () => {
    await assert.rejects(carbon.services.add('notes', component('notes')), {status: 409})
    await assert.rejects(carbon.services.add('channel', component('notes')), {status: 409})
  })

  it('hands a handler its binding in env, under the name its service.toml gives it', async () => {
    await carbon.services.add('stash', component('stash'))
    const stash = await channelIn(personal, {name: 'Stash', services: ['stash']})
    await stash.call('POST [stash]/items', {name: 'kept'})
    assert.deepStrictEqual(await stash.call('GET [stash]/items'), {bindings: ['store'], items: [{name: 'kept'}]})
  })

  it('refuses what is not a component carrying a service.toml, and a package it cannot read', async () => {
    const unpackaged = Uint8Array.from([0, 0x61, 0x73, 0x6d, 0x0d, 0, 1, 0])
    await assert.rejects(carbon.services.add('bare', unpackaged), {status: 400, message: /carries no service.toml/})
    await assert.rejects(carbon.services.add('text', new TextEncoder().encode('name = "text"')), {status: 400, message: /Not a WebAssembly component/})
    await assert.rejects(carbon.services.add('lost', component('lost')), {code: 'ENOENT'})
  })

  it('refuses a component that does not compile once it has tried, leaving its name free', async () => {
    const broken = packageComponent(Uint8Array.from([0, 0x61, 0x73, 0x6d, 0x0d, 0, 1, 0, 0x7f, 1, 0]), 'name = "broken"\n')
    await assert.rejects(carbon.services.add('broken', broken), {status: 400, message: /Not a component this host can run/})
    await assert.rejects(carbon.services.add('broken', broken), {status: 400}, 'refused again, not as a name already taken')
  })

  it('holds the calls to a service until it has compiled, however long that takes against their own timeout, then answers them', async () => {
    // [stash] gives a call 200 ms, less than this process takes to compile it afresh.
    const adding = carbon.services.add('stash', afresh(readFileSync(component('stash'))))
    const stash = await channelIn(personal, {name: 'Stash', services: ['stash']})
    assert.strictEqual((await stash.call('GET [@carbon]/status', {service: 'stash'})).state, 'initializing')
    const [wrote, read] = [stash.call('POST [stash]/items', {name: 'early'}), stash.call('GET [stash]/items')]
    await adding
    await wrote
    assert.deepStrictEqual((await read).items, [{name: 'early'}])
    assert.strictEqual((await stash.call('GET [@carbon]/status', {service: 'stash'})).state, 'idle')
  })

  it('refuses an identity or channel naming a service this device has not registered', async () => {
    await assert.rejects(carbon.call('POST [@carbon]/identity', {slug: 'sally', device: 'x', services: ['crm']}), {status: 400, details: {services: ['crm']}})
    await assert.rejects(personal.call('POST [channel]/services', {name: 'crm'}), {status: 404, details: {service: 'crm'}})
  })

  it('keeps data across a restart that registers the service again, and stops its spaces until it does', async () => {
    await personal.call('POST [notes]/notes', {text: 'kept'})
    const restarted = (await start(config)).ref('identity:alice').ref('personal:alice')
    assert.deepStrictEqual(await texts(restarted), ['kept', 'kept again'])
    const bare = await configure(config)
    const unregistered = bare.ref('identity:alice').ref('personal:alice')
    await assert.rejects(unregistered.call('GET [notes]/notes'), (error) => [error.status, error.code, error.details.failure.code].join() === '503,SERVICE_STOPPED,PACKAGE_UNAVAILABLE')
    assert.strictEqual(unregistered.runState('notes').state, 'stopped')
    assert.deepStrictEqual(unregistered.runState(), {state: 'idle'})
    await bare.services.add('notes', component('notes'))
    assert.deepStrictEqual(await texts(unregistered), ['kept', 'kept again'], 'adding its package initializes it again')
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
    await sleep(20)
    assert.deepStrictEqual(await texts(personal), [])
  })

  it("shows a write the op's time and randomness drawn from the op, and a GET this device's clock", async () => {
    const before = Date.now()
    const written = await personal.call('POST [notes]/clock')
    assert.strictEqual(written.now, written.time, 'Date.now() is the op\'s time')
    assert.ok(written.time >= before && written.random >= 0 && written.random < 1)
    assert.notStrictEqual((await personal.call('POST [notes]/clock')).random, written.random, 'another op draws other randomness')
    assert.ok((await personal.call('GET [notes]/clock')).now >= before)
  })

  it('logs through the host, outside the op', async () => {
    assert.deepStrictEqual(await personal.call('POST [notes]/logged', {text: 'hi'}), {logged: true})
  })

  const refused = [
    {key: 'GET [notes]/writes', status: 400, why: 'a GET calling a write'},
    {key: 'GET [chat]/messages', status: 404, why: 'a service the channel does not name'},
    {key: 'GET [notes]/nothing', status: 404, why: 'a resource the service does not have'},
    {key: 'DELETE [notes]/notes', status: 405, why: 'a method the resource does not answer'},
    {key: 'not a key', status: 400, why: 'something that is not a service key'},
    {key: 'GET [@elsewhere]/x', status: 404, why: 'a local service this device does not run'},
    {key: 'POST [@carbon]/status', status: 404, why: 'a [@carbon] call it does not answer'},
    {key: 'POST [channel]/members', status: 503, why: 'a call that needs peers'},
    {key: 'POST [notes]/reach', status: 503, why: 'an effect a handler makes that needs peers'}
  ]

  for (const {key, status, why} of refused) {
    it(`refuses ${why}`, async () => {
      await assert.rejects(personal.call(key, {}), {status})
    })
  }
})

// The world outside, as the test plays it: an HTTP server that answers
// `/slow` after a pause, `/echo` with what it was sent, and any other path
// with its own name, and keeps what it was asked.
async function outside() {
  const asked = []
  const server = createServer(async (request, response) => {
    let body = ''
    for await (const chunk of request) body += chunk
    asked.push(`${request.method} ${request.url}`)
    if (request.url === '/slow') await sleep(100)
    const answer = request.url === '/echo' ? JSON.stringify({body, type: request.headers['content-type']}) : request.url === '/slow' ? 'slow' : request.url.slice(1)
    response.writeHead(200, {'content-type': request.url === '/echo' ? 'application/json' : 'text/plain'}).end(answer)
  })
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  const url = (path) => `http://127.0.0.1:${server.address().port}${path}`
  return {url, asked, close: () => server.close()}
}

// An address nothing listens on.
async function nowhere() {
  const world = await outside()
  const url = world.url('/gone')
  world.close()
  return url
}

// The address of a server that hangs up on every connection, before saying anything.
async function hangingUp() {
  const server = createTcpServer((socket) => socket.destroy())
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  return {address: `127.0.0.1:${server.address().port}`, close: () => server.close()}
}

describe('fetch', () => {
  let world, fetcher
  before(async () => {
    world = await outside()
  })
  after(() => world.close())
  beforeEach(async () => {
    world.asked.length = 0
    await carbon.services.add('fetcher', component('fetcher'))
    fetcher = await channelIn(personal, {name: 'Outside', services: ['fetcher']})
  })

  it('reaches the world outside from a write, sending its body and headers', async () => {
    const sent = {url: world.url('/echo'), method: 'POST', body: JSON.stringify({héllo: 'wörld'}), headers: {'content-type': 'application/json'}}
    const [answered] = await fetcher.call('POST [fetcher]/fetch', {requests: [sent]})
    assert.deepStrictEqual(answered, {status: 200, ok: true, type: 'application/json', text: JSON.stringify({body: sent.body, type: 'application/json'})})
    assert.deepStrictEqual(world.asked, ['POST /echo'])
  })

  it('sends requests made together at once, each answered with its own response', async () => {
    const answered = await fetcher.call('POST [fetcher]/fetch', {requests: [{url: world.url('/slow')}, {url: world.url('/fast')}]})
    assert.deepStrictEqual(answered.map((answer) => answer.text), ['slow', 'fast'])
  })

  it("keeps running the handler's own code while a request waits for its response", async () => {
    const finished = await fetcher.call('GET [fetcher]/race', {slow: world.url('/slow'), fast: world.url('/fast')})
    assert.deepStrictEqual(finished, ['fast', 'slow'])
  })

  it('reaches it from a GET every time', async () => {
    for (let time = 0; time < 2; time++) assert.deepStrictEqual((await fetcher.call('GET [fetcher]/fetch', {requests: [{url: world.url('/hello')}]}))[0].text, 'hello')
    assert.deepStrictEqual(world.asked, ['GET /hello', 'GET /hello'])
  })

  it('sends an https request over TLS', async () => {
    const server = await hangingUp()
    const [answered] = await fetcher.call('GET [fetcher]/fetch', {requests: [{url: `https://${server.address}/`}]})
    server.close()
    assert.match(answered.error, /^TypeError: fetch failed: .*TLS error/)
  })

  it('rejects with a TypeError when the request fails, as fetch does', async () => {
    const [answered] = await fetcher.call('POST [fetcher]/fetch', {requests: [{url: await nowhere()}]})
    assert.match(answered.error, /^TypeError: fetch failed/)
  })
})

describe('channels', () => {
  it('creates a channel nested in the calling one, with its own slug, services and status', async () => {
    const room = await channelIn(personal, {name: "Randall's Playhouse", services: ['notes']})
    const [listed] = (await personal.call('GET [channel]/refs')).filter((ref) => ref.kind === 'channel')
    assert.deepStrictEqual([listed.slug, listed.channel], ['randalls-playhouse', room.channel])
    assert.strictEqual(personal.ref('channel:randalls-playhouse').channel, room.channel)

    const status = await room.call('GET [@carbon]/status')
    assert.deepStrictEqual(
      {name: status.name, services: status.services, access: status.access, sync: status.sync, peers: status.peers},
      {name: "Randall's Playhouse", services: [{name: 'notes', access: ['manage'], sync: 'off', state: 'idle'}], access: ['manage'], sync: 'off', peers: []}
    )
    await room.call('POST [notes]/notes', {text: 'in the room'})
    assert.deepStrictEqual(await texts(room), ['in the room', 'in the room again'])
    assert.deepStrictEqual(await texts(personal), [])
  })
})

describe('watch', () => {
  // The results a watch delivers, and a way to wait for the next one.
  function watching(channel, key) {
    const results = []
    let next
    const stop = channel.watch(key, (value, error) => {
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
    await sleep(20)
    assert.deepStrictEqual(results, [[]])
  })

  it('stops when asked', async () => {
    const {results, stop, arrival} = watching(personal, 'GET [notes]/notes')
    await arrival()
    stop()
    await personal.call('POST [notes]/notes', {text: 'unseen'})
    await sleep(20)
    assert.deepStrictEqual(results, [[]])
  })

  it('watches a builtin GET too', async () => {
    const {results, arrival} = watching(personal, 'GET [channel]/refs')
    await arrival()
    personal.call('POST [channel]/channel', {name: 'Lobby'})
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

  it('hands a failing GET its error', async () => {
    const {results, arrival} = watching(personal, 'GET [notes]/broken')
    await arrival()
    assert.deepStrictEqual(results, [{error: 500}])
  })
})

describe('timeout', () => {
  it('settles a call its handler has not settled in time, rolling back its writes and dropping its instance', async () => {
    await assert.rejects(personal.call('POST [notes]/spin', {text: 'spun'}), {status: 504, code: 'TIMED_OUT'})
    assert.deepStrictEqual(await texts(personal), [])
    await personal.call('POST [notes]/notes', {text: 'next'})
    assert.deepStrictEqual(await texts(personal), ['next', 'next again'])
  })
})

describe('run state', () => {
  // Hold a service's file from another process, as a second copy of the app
  // would; resolves with a function that lets go of it.
  async function holdLock(service) {
    const path = join(dirname(config), 'data', `${service}.db`)
    const holding = `const db = new (require('node:sqlite').DatabaseSync)(${JSON.stringify(path)}); db.exec('BEGIN IMMEDIATE'); console.log('held'); process.stdin.on('end', () => db.exec('ROLLBACK')); process.stdin.resume()`
    const holder = spawn(process.execPath, ['--no-warnings', '-e', holding], {stdio: ['pipe', 'pipe', 'inherit']})
    await once(holder.stdout, 'data')
    return async () => {
      holder.stdin.end()
      await once(holder, 'exit')
    }
  }

  it('stops a service on a system failure, failing its calls and watches with a 503, until it is resumed', async () => {
    const states = []
    personal.watchRunState('notes', (state) => states.push(state.state))
    const results = []
    let next
    personal.watch('GET [notes]/notes', (value, error) => {
      results.push(error ? {error: error.code} : value)
      next?.()
    })
    const arrival = () => new Promise((resolve) => { next = resolve })
    await arrival()
    assert.deepStrictEqual(personal.runState('notes'), {state: 'idle'})

    const release = await holdLock(await serviceOf(personal, 'notes'))
    const stopped = arrival()
    await assert.rejects(personal.call('POST [notes]/notes', {text: 'lost'}), {status: 503, code: 'SERVICE_STOPPED'})
    await stopped
    assert.deepStrictEqual(results.at(-1), {error: 'SERVICE_STOPPED'})
    const {state, error} = personal.runState('notes')
    assert.deepStrictEqual([state, error.code], ['stopped', 'SYSTEM_FAILURE'])
    await assert.rejects(personal.call('GET [notes]/notes'), {status: 503})

    await release()
    const resumed = arrival()
    personal.resume('notes')
    await resumed
    assert.deepStrictEqual(results.at(-1), [])
    await personal.call('POST [notes]/notes', {text: 'kept'})
    assert.deepStrictEqual(await texts(personal), ['kept', 'kept again'])
    assert.deepStrictEqual(states, ['stopped', 'idle', 'applying', 'idle'])
  })
})

describe('refs', () => {
  it('reach a channel at once, also after a restart', async () => {
    const {channel} = await personal.call('POST [channel]/channel', {name: 'Lobby'})
    assert.strictEqual(personal.ref('channel:lobby').channel, channel)
    const restarted = await start(config)
    assert.strictEqual(restarted.ref('identity:alice').ref('personal:alice').ref('channel:lobby').channel, channel)
  })
})

describe('schema', () => {
  it("is set up by a service's setup, which applies only the migrations not applied yet", async () => {
    await personal.call('POST [notes]/tags', {name: 'red'})
    const restarted = await configure(config)
    await restarted.services.add('notes', component('grown'))
    assert.deepStrictEqual(await restarted.ref('identity:alice').ref('personal:alice').call('GET [notes]/tags'), [{name: 'red', color: 'none'}])
  })

  it('refuses a service whose table takes its keys from SQLite, and says how to fix it', async () => {
    await carbon.services.add('counter', component('counter'))
    await assert.rejects(personal.call('POST [channel]/services', {name: 'counter'}), (error) => {
      assert.deepStrictEqual([error.status, error.details], [400, {table: 'hit'}])
      assert.match(error.message, /must come from the op that creates it.*op_id\(\)/)
      return true
    })
  })

  it("keeps a service out of the core's own tables in its file", async () => {
    await carbon.services.add('snoop', component('snoop'))
    const room = await channelIn(personal, {name: 'Snooped', services: ['snoop']})
    await assert.rejects(room.call('GET [snoop]/log'), {status: 403})
  })
})
