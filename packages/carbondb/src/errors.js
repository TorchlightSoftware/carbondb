// Service errors as they cross from the core: as data in an Error's message,
// or as JSON text from a component's host. `toError` turns whatever was
// thrown back into a ServiceError.

import {deserializeError, isServiceError, toServiceError} from '@tlsft/service-error'

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
  const data = typeof thrown === 'string' ? jsonOf(thrown) : typeof thrown?.message === 'string' ? jsonOf(thrown.message) : undefined
  return isServiceError(data) ? deserializeError(data) : toServiceError(thrown)
}
