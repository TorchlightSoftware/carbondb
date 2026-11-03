// An HTTP adapter for one space. CarbonDB binds no protocol: the app decides
// which space a request reaches and who it acts as, and this adapter turns a
// request into a call on that space's handle, and a service error into a
// response with its status.
//
//   POST /chat/messages        -> POST [chat]/messages, the JSON body as input
//   GET  /chat/messages?x=1    -> GET [chat]/messages, the query as input
//   GET  /watch/chat/messages  -> a stream of server-sent events, one per result of watch

import {badRequest, notFound, serializeError, toServiceError} from '@tlsft/service-error'

async function bodyOf(request) {
  const chunks = []
  for await (const chunk of request) chunks.push(chunk)
  const text = Buffer.concat(chunks).toString('utf8')
  if (!text) return null
  try {
    return JSON.parse(text)
  } catch {
    throw badRequest('The request body is not JSON')
  }
}

// The service key a request names: its first path segment is the service.
function keyOf(method, pathname) {
  const [service, ...resource] = pathname.split('/').filter(Boolean)
  if (!service) throw notFound('Name a service: /<service>/<resource>')
  return `${method} [${service}]/${resource.join('/')}`.replace(/\/$/, '')
}

function send(response, status, body) {
  response.writeHead(status, {'content-type': 'application/json'})
  response.end(JSON.stringify(body ?? null))
}

// Stream a watched GET as server-sent events until the client goes away.
function stream(space, request, response, pathname) {
  const key = keyOf('GET', pathname.slice('/watch'.length))
  response.writeHead(200, {'content-type': 'text/event-stream', 'cache-control': 'no-cache'})
  const stop = space.watch(key, (value, error) => {
    response.write(`data: ${JSON.stringify(error ? {error: serializeError(error)} : {value})}\n\n`)
  })
  request.on('close', stop)
}

// A request listener answering every request with a call on `space`.
export function serveSpace(space) {
  return async (request, response) => {
    const {pathname, searchParams} = new URL(request.url, 'http://localhost')
    if (request.method === 'GET' && pathname.startsWith('/watch/')) return stream(space, request, response, pathname)
    try {
      const input = request.method === 'GET' ? Object.fromEntries(searchParams) : await bodyOf(request)
      send(response, 200, await space.call(keyOf(request.method, pathname), input))
    } catch (thrown) {
      const error = toServiceError(thrown)
      send(response, error.status, serializeError(error))
    }
  }
}
