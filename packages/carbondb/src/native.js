// The seam to the Rust core. The core answers in callback style, and every
// error it raises crosses as service error data: in an Error's message when a
// call into it throws, or as a callback's first argument. `toError` and
// `answered` turn both back into ServiceErrors.

import {createRequire} from 'node:module'
import {deserializeError, isServiceError, toServiceError} from '@tlsft/service-error'

const binding = createRequire(import.meta.url)('../carbondb.node')

export const NativeCarbon = binding.Carbon
export const hash = binding.hash

const jsonOf = (text) => {
  try {
    return JSON.parse(text)
  } catch {
    return undefined
  }
}

// Whatever was thrown, as a ServiceError.
export function toError(thrown) {
  if (isServiceError(thrown)) return toServiceError(thrown)
  const data = typeof thrown?.message === 'string' ? jsonOf(thrown.message) : undefined
  return isServiceError(data) ? deserializeError(data) : toServiceError(thrown)
}

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
