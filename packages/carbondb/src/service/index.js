// A JavaScript service as a WebAssembly component: `carbondb/service` runs
// inside the component, and turns a service's module into the exports of the
// `carbondb:service` world. `carbondb/build` generates the entry that does so;
// a service only exports its `calls`, keyed by service key, each with a
// `handler`, and an optional `setup`:
//
//   export const {setup, handle} = serve(service)
//
// Each call arrives as `handle`'s request. The handler receives its path
// parameters and input; `env`, its `sql` binding under the name its
// service.toml gives it, as a Worker's env holds it; `call`, `callsync` and
// `effect`; and the op, when the call writes. Everything reaches the core through the
// world's host imports, and a failure crosses as a service error's JSON.
// `console` logs through the host, and `fetch` reaches the outside world
// through the core (./fetch.js).

import host from 'carbondb:service/host@0.1.0'
import {serializeError} from '@tlsft/service-error'
import {toError} from '../errors.js'
import {Headers, Response, fetch} from './fetch.js'
import {compileRoutes, route} from '../routes.js'

// The world's exports for a service's module.
export function serve(service) {
  if (service.setup !== undefined && typeof service.setup !== 'function') throw new TypeError('A service declares a setup that is not a function')
  const routes = compileRoutes(service.calls ?? {})
  globalThis.console = consoleOf()
  Object.assign(globalThis, {fetch, Headers, Response})
  return {
    // The service's `setup`.
    setup: () => crossing(() => {
      const returned = service.setup?.(dbOf(true))
      if (typeof returned?.then === 'function') throw new TypeError('A service has a setup that returns a Promise; setup finishes before it returns')
    }),
    handle: async (text) => {
      const request = JSON.parse(text)
      try {
        const {call, path} = route(routes, request.method, request.resource)
        return JSON.stringify((await call.handler(handlerContextOf(request, path))) ?? null)
      } catch (thrown) {
        throw failure(thrown)
      }
    }
  }
}

// A failure as it crosses back to the core: a service error's JSON.
const failure = (thrown) => JSON.stringify(serializeError(toError(thrown)))

// Run something, throwing any failure across as a service error's JSON.
function crossing(work) {
  try {
    return work()
  } catch (thrown) {
    throw failure(thrown)
  }
}

// Call the host, turning a failure it answers with back into a ServiceError.
function reaching(work) {
  try {
    return work()
  } catch (thrown) {
    throw toError(thrown)
  }
}

async function reachingLater(work) {
  try {
    return await work()
  } catch (thrown) {
    throw toError(thrown)
  }
}

// The `sql` binding: queries always, and `run` when the call writes.
function dbOf(writes) {
  const all = (sql, params = []) => JSON.parse(reaching(() => host.all(sql, JSON.stringify(params))))
  const db = {all, get: (sql, params) => all(sql, params)[0] ?? null}
  if (writes) db.run = (sql, params = []) => Number(reaching(() => host.run(sql, JSON.stringify(params))))
  return db
}

// What a handler receives. `Math.random` draws from the op, as `op.random`
// does, so every replica draws the same; a GET, which no replica repeats,
// draws from a generator seeded by this device's clock.
function handlerContextOf({input, op, binding}, path) {
  Math.random = op ? opOf(op, path).random : seeded(Date.now())
  const across = (reach) => (key, callInput = null) => reachingLater(async () => JSON.parse(await reach(key, JSON.stringify(callInput))))
  const [call, callsync, effect] = [across(host.call), across(host.callsync), across(host.effect)]
  const env = binding ? {[binding]: dbOf(Boolean(op))} : {}
  const context = {path, input, env, call, callsync, effect}
  return op ? {...context, op: opOf(op, path)} : context
}

// An op as a handler sees it: who made it, when, and the ids and randomness
// it draws, from the core, so every replica and every call within the op
// draws from one sequence.
function opOf(op, path) {
  const draw = () => reaching(() => host.draw())
  return Object.freeze({
    key: op.key, path, input: op.input, time: op.time, hash: op.hash, version: op.version,
    author: op.author,
    device: op.device,
    id: () => draw().slice(0, 32),
    random: () => parseInt(draw().slice(0, 13), 16) / 2 ** 52
  })
}

// Numbers in [0, 1) from a seed: mulberry32.
function seeded(seed) {
  let state = seed >>> 0
  return () => {
    state = (state + 0x6d2b79f5) >>> 0
    let t = Math.imul(state ^ (state >>> 15), state | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

// `console`, logging through the host: whoever runs the device sees it, never the op.
function consoleOf() {
  const text = (values) => values.map((value) => (typeof value === 'string' ? value : stringify(value))).join(' ')
  const at = (level) => (...values) => host.log(level, text(values))
  return {debug: at('debug'), log: at('info'), info: at('info'), warn: at('warn'), error: at('error')}
}

function stringify(value) {
  if (value instanceof Error) return value.stack ?? `${value.name}: ${value.message}`
  try {
    return JSON.stringify(value) ?? String(value)
  } catch {
    return String(value)
  }
}
