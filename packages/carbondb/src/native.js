// The seam to the Rust core, which runs on a thread of its own. Calls into it
// that answer at once return or throw here; everything it answers later, a
// call's answer or a watch's, crosses back through one courier, in the order
// the core gave them, to the callback JS registered for it by id. The courier
// keeps Node running while anything is owed an answer: a call until it
// answers, a watch until its first answer, since every later one follows a
// commit some call kept Node running for. Every error the core raises crosses
// as service error data: in an Error's message, or as a callback's first
// argument. `toError` and `answered` turn both back into ServiceErrors.

import {createRequire} from 'node:module'
import {deserializeError} from '@tlsft/service-error'
import {toError} from './errors.js'

const binding = createRequire(import.meta.url)('../carbondb.node')

// Embed a service.toml in a built component: a package, ready to register.
export const packageComponent = binding.packageComponent

// Call the core synchronously, rethrowing its errors as ServiceErrors.
export function core(work) {
  try {
    return work()
  } catch (thrown) {
    throw toError(thrown)
  }
}

// Start something the core answers through a callback, `(error, value)`, and
// settle with its answer.
export function answered(start) {
  return new Promise((resolve, reject) => {
    core(() => start((error, value) => (error ? reject(deserializeError(error)) : resolve(value))))
  })
}

// The callbacks the core owes answers to, by id, and whether Node must keep
// running for them.
class Courier {
  #next = 0
  #callbacks = new Map()
  #owed = new Set()
  #keep

  // `keep(true)` keeps Node running, `keep(false)` lets it stop.
  constructor(keep) {
    this.#keep = keep
  }

  // An id for a callback answered once, owed its answer.
  once(callback) {
    return this.#register({callback, once: true}, true)
  }

  // An id for a callback answered again and again; `owed` its first answer.
  many(callback, owed) {
    return this.#register({callback, once: false}, owed)
  }

  #register(entry, owed) {
    const id = this.#next++
    this.#callbacks.set(id, entry)
    if (owed) this.#owe(id)
    return id
  }

  deliver(id, error, value) {
    const entry = this.#callbacks.get(id)
    if (!entry) return
    if (entry.once) this.#callbacks.delete(id)
    this.#settle(id)
    entry.callback(error, value)
  }

  // Forget a callback, owed nothing more.
  forget(id) {
    this.#callbacks.delete(id)
    this.#settle(id)
  }

  #owe(id) {
    if (this.#owed.size === 0) this.#keep(true)
    this.#owed.add(id)
  }

  #settle(id) {
    if (this.#owed.delete(id) && this.#owed.size === 0) this.#keep(false)
  }
}

// The core as the handles reach it: everything it answers later takes a
// callback, `(error, value)`, or a RunState listener, `(state)`.
export class NativeCarbon {
  #native
  #courier
  #watched = new Map()

  static open(path) {
    const carbon = new NativeCarbon()
    carbon.#native = binding.Carbon.open(path, (id, error, value) => carbon.#courier.deliver(id, error, value))
    carbon.#courier = new Courier((keep) => carbon.#native.keep(keep))
    return carbon
  }

  register(name, bytes) {
    this.#native.register(name, bytes)
  }

  call(channel, identity, key, input, reply) {
    const id = this.#courier.once(reply)
    this.#dispatch(id, () => this.#native.call(channel, identity, key, input, id))
  }

  watch(channel, identity, key, listener) {
    const id = this.#courier.many(listener, true)
    return this.#watching(id, () => this.#native.watch(channel, identity, key, id))
  }

  watchRunState(space, listener) {
    const id = this.#courier.many((_, state) => listener(state), false)
    return this.#watching(id, () => this.#native.watchRunState(space, id))
  }

  unwatch(watch) {
    this.#native.unwatch(watch)
    this.#courier.forget(this.#watched.get(watch))
    this.#watched.delete(watch)
  }

  findRef(channel, kind, slug) {
    return this.#native.findRef(channel, kind, slug)
  }

  service(channel, name) {
    return this.#native.service(channel, name)
  }

  runState(space) {
    return this.#native.runState(space)
  }

  resume(space, reply) {
    const id = this.#courier.once(reply)
    this.#dispatch(id, () => this.#native.resume(space, id))
  }

  identity(slug) {
    return this.#native.identity(slug)
  }

  // Start something the core answers by `id`, forgetting `id` when it throws instead.
  #dispatch(id, start) {
    try {
      return start()
    } catch (thrown) {
      this.#courier.forget(id)
      throw thrown
    }
  }

  #watching(id, start) {
    const watch = this.#dispatch(id, start)
    this.#watched.set(watch, id)
    return watch
  }
}
