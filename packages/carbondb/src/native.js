// The seam to the Rust core. Every error the core raises crosses as a JSON
// service error in its message; `settle` turns it back into a ServiceError. A
// handler's own error crosses untouched and keeps its meaning.

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
export function settle(thrown) {
  if (isServiceError(thrown)) return toServiceError(thrown)
  const data = typeof thrown?.message === 'string' ? jsonOf(thrown.message) : undefined
  return isServiceError(data) ? deserializeError(data) : toServiceError(thrown)
}

// Call the core synchronously, rethrowing its errors as ServiceErrors.
export function core(work) {
  try {
    return work()
  } catch (thrown) {
    throw settle(thrown)
  }
}

// Await the core, rejecting with ServiceErrors.
export const coreAsync = (promise) => promise.catch((thrown) => { throw settle(thrown) })
