// An npm service's entry point. The core routes every call to a service by
// its name and hands it to the service's entry point as an invocation; this
// one routes the call to its handler by method and resource, and settles the
// invocation once, with what the handler returns or throws.

import {serializeError} from '@tlsft/service-error'
import {answered, core, toError} from './native.js'
import {parseKey, route} from './routes.js'

export function entryOf(routes) {
  return (invocation) => {
    const request = invocation.request()
    handle(routes, invocation, request).then(
      (value) => invocation.settle(null, value ?? null),
      (thrown) => invocation.settle(serializeError(toError(thrown)), null)
    )
  }
}

// The handler's result, or its rejection, including a call that reaches no handler.
async function handle(routes, invocation, request) {
  const {call, path} = route(routes, request.method, request.resource)
  return call.handler(contextOf(invocation, request, path))
}

// What a handler receives: its path parameters and input, its queries, and a
// way to make calls of its own; and the op, with `run`, when the call writes.
function contextOf(invocation, {input, op}, path) {
  const queries = {
    get: (sql, params) => core(() => invocation.get(sql, params)),
    all: (sql, params) => core(() => invocation.all(sql, params))
  }
  const call = (key, callInput = null) => answered((reply) => invocation.call(parseKey(key), callInput, reply))
  if (!op) return {path, input, db: queries, call}
  const db = {...queries, run: (sql, params) => core(() => invocation.run(sql, params))}
  return {path, input, db, call, op: opContext(invocation, op, path)}
}

// An op's context: who made it, when, and the ids and randomness a handler
// takes from it, drawn from the op's hash so every replica computes the same.
function opContext(invocation, op, path) {
  const draw = () => core(() => invocation.draw())
  return Object.freeze({
    key: op.key, path, input: op.input, time: op.time, hash: op.hash,
    author: op.author,
    device: op.device,
    id: () => draw().slice(0, 32),
    random: () => parseInt(draw().slice(0, 13), 16) / 2 ** 52
  })
}
