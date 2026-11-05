// One space on this device: its database in the core, the services its origin
// names, and the watches on it. A call is routed by its service name, to a
// service the core answers or to an app service's handler, and runs as a job
// on the side of the space it needs, in the order the core's queue gives it.

import {parseServiceKey} from '@tlsft/service-key'
import {badRequest, notFound} from '@tlsft/service-error'
import {core, coreAsync, hash} from './native.js'
import {route} from './routes.js'
import {BUILTIN_SERVICES} from './services.js'

export function parseKey(key) {
  const parts = parseServiceKey(key)
  if (!parts) throw badRequest(`'${key}' is not a service key`)
  return parts
}

// Whether the core answers a call itself: any local `[@name]`, or a replicated builtin.
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

// A handler's queries on one side of the space's database: the writer inside a
// write's transaction, the reader on a GET's snapshot. Only a write's db adds `run`.
const queries = (native, side) => ({
  get: (sql, params) => native.get(side, sql, params),
  all: (sql, params) => native.all(side, sql, params)
})

// The side of the space a call runs on: GETs read a snapshot, everything else writes.
const sideOf = (parts) => parts.method === 'GET' ? 'reader' : 'writer'

export class Space {
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
    const side = sideOf(parts)
    const job = isBuiltin(parts) ? this.#builtinJob(native, actor, parts, input) : this.#serviceJob(native, actor, key, parts, input)
    const outcome = await this.#turn(native, side, job)
    if (side === 'writer') this.#changed(outcome.tables)
    return outcome
  }

  // Run a job when its turn on this side of the space comes, and settle with
  // what it returns. A job never throws into the core.
  #turn(native, side, job) {
    return new Promise((resolve, reject) => native.enqueue(side, () => {
      try {
        resolve(job())
      } catch (thrown) {
        reject(thrown)
      }
    }))
  }

  // A job for a call the core answers.
  #builtinJob(native, actor, parts, input) {
    if (parts.service === 'carbon' && parts.method === 'POST' && parts.resource === '/space') this.carbon.services.requireRegistered(input?.services)
    return () => core(() => this.carbon.native.callSpace(native, actor, parts, input))
  }

  // A job for a call to an app service, routed to its handler. A write takes
  // its op's time when its turn comes, so it follows everything applied before it.
  #serviceJob(native, actor, key, parts, input) {
    const {call, path} = route(this.#registered(parts).routes, parts.method, parts.resource)
    if (sideOf(parts) === 'reader') {
      const db = queries(native, 'reader')
      return () => coreAsync(native.read(async () => call.handler({path, input, db})))
    }
    const db = {...queries(native, 'writer'), run: (sql, params) => native.run(sql, params)}
    return () => {
      const op = opContext(this.carbon, actor, key, path, input)
      return coreAsync(native.apply(async () => call.handler({path, input, op, db})))
    }
  }

  #registered(parts) {
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
