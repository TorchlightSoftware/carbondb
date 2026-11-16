// Handles are how an app reaches channels: one per channel, reached through
// refs from `carbon`, the system handle. A handle also carries the identity
// acting through it, set when the ref chain passes through an identity. A
// call on a channel's handle reaches the service the channel names, as
// `[chat]`. The core does the work: a handle names its channel and identity,
// and hands the app each answer a watch gets.

import {deserializeError, notFound} from '@tlsft/service-error'
import {answered, core} from './native.js'

// `kind:slug`, as in `team:sales`.
function parseName(name) {
  const [kind, slug, ...rest] = String(name).split(':')
  if (!kind || !slug || rest.length) throw new TypeError(`'${name}' is not a channel name, which is kind:slug`)
  return {kind, slug}
}

// Hand the app an answer on a turn of its own, as a Promise would: an app
// function that throws surfaces as an uncaught error, and holds no call back.
const deliver = (fn, ...answer) => queueMicrotask(() => fn(...answer))

export class ChannelHandle {
  #native

  constructor(native, channel, identity) {
    this.#native = native
    this.channel = channel
    this.identity = identity
  }

  // The identity acting through this handle, as the core takes it.
  #acting() {
    return this.identity?.id ?? null
  }

  // The handle of a channel a ref here points to, found at once.
  ref(name) {
    const {kind, slug} = parseName(name)
    const found = core(() => this.#native.findRef(this.channel, kind, slug))
    return new ChannelHandle(this.#native, found.channel, this.identity)
  }

  // A call's value, once the watches its commit touched have run again.
  call(key, input = null) {
    return answered((reply) => this.#native.call(this.channel, this.#acting(), key, input, reply))
  }

  // Run a GET now and again after every commit that writes a table it read.
  // `fn` receives each result, or undefined and the error when the GET fails.
  // Returns a function that stops it.
  watch(key, fn) {
    const id = core(() => this.#native.watch(this.channel, this.#acting(), key, (error, value) => (error ? deliver(fn, undefined, deserializeError(error)) : deliver(fn, value))))
    return () => this.#native.unwatch(id)
  }

  // The RunState of a service this channel names, or of the channel itself:
  // initializing, idle, applying an op, or stopped by a system failure.
  runState(service = 'channel') {
    return core(() => this.#native.runState(this.#service(service)))
  }

  // Call `fn` with each change of a service's RunState; returns a function that stops it.
  watchRunState(service, fn) {
    const id = core(() => this.#native.watchRunState(this.#service(service), (state) => deliver(fn, state)))
    return () => this.#native.unwatch(id)
  }

  // Resume a service once the system failure that stopped it is fixed by hand.
  resume(service = 'channel') {
    core(() => this.#native.resume(this.#service(service)))
  }

  // The id of a service this channel names, by its name.
  #service(name) {
    return core(() => this.#native.service(this.channel, name))
  }
}

// The system handle, `carbon`: this device's root, where identities live.
export class SystemHandle {
  #native

  constructor(native, services) {
    this.#native = native
    this.services = services
  }

  // The handle of an identity's own channel on this device, which acts as that identity.
  ref(name) {
    const {kind, slug} = parseName(name)
    if (kind !== 'identity') throw notFound(`The system holds identities only, not ${name}`)
    const identity = core(() => this.#native.identity(slug))
    return new ChannelHandle(this.#native, identity.channel, identity)
  }

  // A call on the system handle, which runs only `[@carbon]`: `POST
  // [@carbon]/identity` creates an identity, `GET [@carbon]/identities` lists
  // them, and status, sync and data name a channel or service in their input.
  call(key, input = null) {
    return answered((reply) => this.#native.call(null, null, key, input, reply))
  }
}
