// A service's calls as routes. A call is matched on its method and resource;
// the service name written in a declared key is ignored, because the app names
// the service when it registers it.

import {parseServiceKey} from '@tlsft/service-key'
import {methodNotAllowed, notFound} from '@tlsft/service-error'

const segmentsOf = (resource) => resource.split('/').filter(Boolean)

// A service's declared calls as routes, or a TypeError for a malformed declaration.
export function compileRoutes(calls) {
  return Object.entries(calls).map(([key, call]) => {
    const parts = parseServiceKey(key)
    if (!parts?.method) throw new TypeError(`'${key}' is not a service key with a method`)
    if (typeof call?.handler !== 'function') throw new TypeError(`${key} has no handler`)
    return {method: parts.method, segments: segmentsOf(parts.resource), call}
  })
}

// The path parameters a route's segments capture from a resource, or null when it doesn't match.
function capture(segments, actual) {
  if (segments.length !== actual.length) return null
  const path = {}
  for (const [index, segment] of segments.entries()) {
    if (segment.startsWith(':')) path[segment.slice(1)] = decodeURIComponent(actual[index])
    else if (segment !== actual[index]) return null
  }
  return path
}

// The call a method and resource reach, with the path parameters it captured.
export function route(routes, method, resource) {
  const actual = segmentsOf(resource)
  const matches = routes.map((candidate) => ({candidate, path: capture(candidate.segments, actual)})).filter(({path}) => path)
  const found = matches.find(({candidate}) => candidate.method === method)
  if (found) return {call: found.candidate.call, path: found.path}
  if (matches.length) throw methodNotAllowed(`${resource} does not answer ${method}`, {methods: matches.map(({candidate}) => candidate.method)})
  throw notFound(`${resource} is not a resource of this service`)
}
