// Handles are how an app reaches spaces: one per space, reached through refs
// from `carbon`, the system handle. A handle also carries the identity acting
// through it, set when the ref chain passes through an identity. The core
// does the work: a handle names its space and identity, and hands the app
// each answer a watch gets.

import {deserializeError, notFound} from '@tlsft/service-error'
import {answered, core} from './native.js'

// `kind:slug`, as in `team:sales`.
function parseName(name) {
  const [kind, slug, ...rest] = String(name).split(':')
  if (!kind || !slug || rest.length) throw new TypeError(`'${name}' is not a space name, which is kind:slug`)
  return {kind, slug}
}

// Hand the app an answer on a turn of its own, as a Promise would: an app
// function that throws surfaces as an uncaught error, and holds no call back.
const deliver = (fn, ...answer) => queueMicrotask(() => fn(...answer))

export class SpaceHandle {
  #native

  constructor(native, space, identity) {
    this.#native = native
    this.space = space
    this.identity = identity
  }

  // The identity acting through this handle, as the core takes it.
  #acting() {
    return this.identity?.id ?? null
  }

  // The handle of a space a ref here points to, found at once.
  ref(name) {
    const {kind, slug} = parseName(name)
    const found = core(() => this.#native.findRef(this.space, kind, slug))
    return new SpaceHandle(this.#native, found.space, this.identity)
  }

  // A call's value, once the watches its commit touched have run again.
  call(key, input = null) {
    return answered((reply) => this.#native.call(this.space, this.#acting(), key, input, reply))
  }

  // Run a GET now and again after every commit that writes a table it read.
  // `fn` receives each result, or undefined and the error when the GET fails.
  // Returns a function that stops it.
  watch(key, fn) {
    const id = core(() => this.#native.watch(this.space, this.#acting(), key, (error, value) => (error ? deliver(fn, undefined, deserializeError(error)) : deliver(fn, value))))
    return () => this.#native.unwatch(id)
  }

  // This space's RunState: initializing, idle, applying an op, or stopped by a system failure.
  runState() {
    return core(() => this.#native.runState(this.space))
  }

  // Call `fn` with each change of this space's RunState; returns a function that stops it.
  watchRunState(fn) {
    const id = core(() => this.#native.watchRunState(this.space, (state) => deliver(fn, state)))
    return () => this.#native.unwatch(id)
  }

  // Resume this space once the system failure that stopped it is fixed by hand.
  resume() {
    core(() => this.#native.resume(this.space))
  }

  // Create a space nested in this one, and return its handle.
  async createSpace(input) {
    const created = await this.call('POST [@carbon]/space', input)
    return new SpaceHandle(this.#native, created.space, this.identity)
  }

  listSpaces() {
    return this.call('GET [space]/refs')
  }

  status() {
    return this.call('GET [@carbon]/status')
  }
}

// The system handle, `carbon`: this device's root, where identities live.
export class SystemHandle {
  #native

  constructor(native, services) {
    this.#native = native
    this.services = services
  }

  // The handle of an identity's space on this device, which acts as that identity.
  ref(name) {
    const {kind, slug} = parseName(name)
    if (kind !== 'identity') throw notFound(`The system holds identities only, not ${name}`)
    const identity = core(() => this.#native.identity(slug))
    return new SpaceHandle(this.#native, identity.space, identity)
  }

  // A call on the system handle, the root space, which runs only local services.
  call(key, input = null) {
    return answered((reply) => this.#native.call(null, null, key, input, reply))
  }

  // An identity, its first device and its personal space, which runs the services named.
  async createIdentity({slug, device, services = []}) {
    const identity = await this.call('POST [@carbon]/identity', {slug, device, services})
    return {
      identity: new SpaceHandle(this.#native, identity.space, identity),
      personal: new SpaceHandle(this.#native, identity.personal, identity)
    }
  }

  resumeIdentity(slug) {
    return this.ref(`identity:${slug}`)
  }

  listIdentities() {
    return this.call('GET [@carbon]/identities')
  }
}
