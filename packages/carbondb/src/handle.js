// Handles are how an app reaches spaces: one per space, reached through refs
// from `carbon`, the system handle. A handle also carries the identity acting
// through it, set when the ref chain passes through an identity.

import {notFound} from '@tlsft/service-error'
import {answered, core} from './native.js'
import {parseKey} from './routes.js'

// `kind:slug`, as in `team:sales`.
function parseName(name) {
  const [kind, slug, ...rest] = String(name).split(':')
  if (!kind || !slug || rest.length) throw new TypeError(`'${name}' is not a space name, which is kind:slug`)
  return {kind, slug}
}

// The acting identity as the core takes it.
const actorOf = (identity) => ({identity: identity?.id ?? null, device: identity?.device ?? null})

export class SpaceHandle {
  #carbon

  constructor(carbon, space, identity) {
    this.#carbon = carbon
    this.space = space
    this.identity = identity
  }

  #runtime() {
    return this.#carbon.space(this.space)
  }

  // The handle of a space a ref here points to.
  ref(name) {
    const {kind, slug} = parseName(name)
    const found = this.#runtime().findRef(kind, slug)
    if (!found) throw notFound(`No ref to ${name} here`)
    return new SpaceHandle(this.#carbon, found.space, this.identity)
  }

  call(key, input) {
    return this.#runtime().call(actorOf(this.identity), key, input)
  }

  watch(key, fn) {
    return this.#runtime().watch(actorOf(this.identity), key, fn)
  }

  // Create a space nested in this one, and return its handle.
  async createSpace(input) {
    const created = await this.call('POST [@carbon]/space', input)
    return new SpaceHandle(this.#carbon, created.space, this.identity)
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
  #carbon

  constructor(carbon) {
    this.#carbon = carbon
  }

  get services() {
    return this.#carbon.services
  }

  // The handle of an identity's space on this device, which acts as that identity.
  ref(name) {
    const {kind, slug} = parseName(name)
    if (kind !== 'identity') throw notFound(`The system holds identities only, not ${name}`)
    const identity = core(() => this.#carbon.native.identity(slug))
    return new SpaceHandle(this.#carbon, identity.space, identity)
  }

  // A call on the system handle, the root space, which runs only local services.
  async call(key, input = null) {
    const parts = parseKey(key)
    const outcome = await answered((reply) => this.#carbon.native.call(null, actorOf(null), parts, input, reply))
    return outcome.value
  }

  // An identity, its first device and its personal space, which runs the services named.
  async createIdentity({slug, device, services = []}) {
    const identity = await this.call('POST [@carbon]/identity', {slug, device, services})
    return {
      identity: new SpaceHandle(this.#carbon, identity.space, identity),
      personal: new SpaceHandle(this.#carbon, identity.personal, identity)
    }
  }

  resumeIdentity(slug) {
    return this.ref(`identity:${slug}`)
  }

  listIdentities() {
    return this.call('GET [@carbon]/identities')
  }
}
