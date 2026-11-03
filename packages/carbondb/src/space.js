// One space on this device: its database in the core, the services its origin
// names, and the watches on it. Calls reach either a builtin service in the
// core or an app service's handler, which runs in a transaction the core holds.

import {parseServiceKey} from '@tlsft/service-key'
import {badRequest, notFound} from '@tlsft/service-error'
import {core, coreAsync, hash} from './native.js'
import {Queue} from './queue.js'
import {route} from './routes.js'
import {BUILTIN_SERVICES} from './services.js'

export function parseKey(key) {
  const parts = parseServiceKey(key)
  if (!parts) throw badRequest(`'${key}' is not a service key`)
  return parts
}

// Whether CarbonDB answers a call itself: `[@carbon]`, or a replicated builtin.
const isBuiltin = (parts) => parts.confined || BUILTIN_SERVICES.includes(parts.service)

// An op's context: who made it, when, and the ids and randomness a handler
// takes from it, derived from the op's hash so every replica computes the same.
function opContext(carbon, actor, key, path, input) {
  const {time, hash: opHash} = carbon.native.stamp({key, input, author: actor.identity, device: actor.device})
  let drawn = 0
  const draw = () => hash({op: opHash, n: drawn++})
  return Object.freeze({
    key, path, input, time, hash: opHash,
    author: actor.identity,
    device: actor.device,
    id: () => draw().slice(0, 32),
    random: () => parseInt(draw().slice(0, 13), 16) / 2 ** 52
  })
}

export class Space {
  #writes = new Queue()
  #reads = new Queue()
  #watches = new Set()
  #flushing = false
  #opened = null

  constructor(carbon, id) {
    this.carbon = carbon
    this.id = id
  }

  // The space's database and origin, opened on first use, with each service's
  // migrations run. A space naming a service this device has not registered is refused.
  #open() {
    if (this.#opened) return this.#opened
    const native = core(() => this.carbon.native.openSpace(this.id))
    const origin = core(() => native.origin())
    const services = new Map(origin.services.filter((name) => !BUILTIN_SERVICES.includes(name)).map((name) => [name, this.carbon.services.get(name)]))
    for (const service of services.values()) core(() => native.migrate(service.name, service.migrations))
    this.#opened = {native, origin, services}
    return this.#opened
  }

  get native() {
    return this.#open().native
  }

  // A sync lookup of the space a ref here points to.
  findRef(kind, slug) {
    return core(() => this.native.findRef(kind, slug))
  }

  async call(actor, key, input) {
    return (await this.#request(actor, key, input)).value
  }

  // A call's outcome: its value, and the tables it read or wrote.
  async #request(actor, key, input = null) {
    const parts = parseKey(key)
    const {native} = this.#open()
    if (isBuiltin(parts)) return this.#builtin(native, actor, parts, input)
    const service = this.#service(parts)
    const {call, path} = route(service.routes, parts.method, parts.resource)
    if (parts.method === 'GET') {
      const db = {get: (sql, params) => native.readGet(sql, params), all: (sql, params) => native.readAll(sql, params)}
      return this.#reads.push(() => coreAsync(native.read(async () => call.handler({path, input, db}))))
    }
    const op = opContext(this.carbon, actor, key, path, input)
    const db = {run: (sql, params) => native.run(sql, params), get: (sql, params) => native.get(sql, params), all: (sql, params) => native.all(sql, params)}
    const outcome = await this.#writes.push(() => coreAsync(native.apply(async () => call.handler({path, input, op, db}))))
    this.#changed(outcome.tables)
    return outcome
  }

  async #builtin(native, actor, parts, input) {
    if (parts.confined && parts.service !== 'carbon') throw notFound(`No local service [@${parts.service}] on this device`)
    if (parts.service === 'carbon' && parts.method === 'POST' && parts.resource === '/space') this.carbon.services.requireRegistered(input?.services)
    const queue = parts.method === 'GET' ? this.#reads : this.#writes
    const outcome = await queue.push(() => core(() => this.carbon.native.callSpace(native, actor, parts, input)))
    if (outcome.wrote) this.#changed(outcome.tables)
    return outcome
  }

  #service(parts) {
    const {services, origin} = this.#open()
    if (!origin.services.includes(parts.service)) throw notFound(`[${parts.service}] does not run in this space`)
    return services.get(parts.service)
  }

  // Run a GET now and again after every commit that writes a table it read.
  // `fn` receives each result, or undefined and the error when the GET fails.
  watch(actor, key, fn) {
    const watch = {actor, key, fn, tables: new Set(), dirty: true, running: false, stopped: false}
    this.#watches.add(watch)
    this.#rerun(watch)
    return () => {
      watch.stopped = true
      this.#watches.delete(watch)
    }
  }

  // Mark the watches a commit affects, and re-run them once the current batch of calls has settled.
  #changed(tables) {
    const written = new Set(tables)
    for (const watch of this.#watches) {
      if ([...watch.tables].some((table) => written.has(table))) watch.dirty = true
    }
    if (this.#flushing) return
    this.#flushing = true
    setImmediate(() => {
      this.#flushing = false
      for (const watch of this.#watches) this.#rerun(watch)
    })
  }

  async #rerun(watch) {
    if (!watch.dirty || watch.running || watch.stopped) return
    watch.dirty = false
    watch.running = true
    const {value, tables, error} = await this.#request(watch.actor, watch.key).catch((thrown) => ({error: thrown}))
    watch.running = false
    if (watch.stopped) return
    if (tables) watch.tables = new Set(tables)
    watch.fn(value, error)
    this.#rerun(watch)
  }
}
