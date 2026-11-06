// An npm service's entry point. The core routes every call to a service by
// its name and hands it to the service's entry point as a handler context;
// this one routes the call to its handler by method and resource, and settles
// the context once, with what the handler returns or throws.

import {serializeError} from '@tlsft/service-error'
import {answered, core, toError} from './native.js'
import {route} from './routes.js'

export function entryOf(routes) {
  return (context) => {
    const request = context.request()
    handle(routes, context, request).then(
      (value) => context.settle(null, value ?? null),
      (thrown) => context.settle(serializeError(toError(thrown)), null)
    )
  }
}

// The handler's result, or its rejection, including a call that reaches no handler.
async function handle(routes, context, request) {
  const {call, path} = route(routes, request.method, request.resource)
  return call.handler(handlerContextOf(context, request, path))
}

// What a handler receives: its path parameters and input, its `db` binding,
// and a way to make calls of its own; and the op, with `run`, when the call writes.
function handlerContextOf(context, {input, op}, path) {
  const queries = {
    get: (sql, params) => core(() => context.get(sql, params)),
    all: (sql, params) => core(() => context.all(sql, params))
  }
  const call = (key, callInput = null) => answered((reply) => context.call(key, callInput, reply))
  if (!op) return {path, input, db: queries, call}
  const db = {...queries, run: (sql, params) => core(() => context.run(sql, params))}
  return {path, input, db, call, op: opOf(context, op, path)}
}

// An op as a handler sees it: who made it, when, and the ids and randomness a
// handler takes from it, drawn from the op's hash so every replica computes the same.
function opOf(context, op, path) {
  return Object.freeze({
    key: op.key, path, input: op.input, time: op.time, hash: op.hash, version: op.version,
    author: op.author,
    device: op.device,
    id: () => core(() => context.id()),
    random: () => core(() => context.random())
  })
}
