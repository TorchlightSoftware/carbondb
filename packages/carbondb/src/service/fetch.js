// `fetch`, as a service inside a component calls it, over the `wasi:http`
// 0.3 client its host links, whose `send` is async: a request in flight
// leaves the handler's other work running. Every request is the core's
// `fetch` effect: in a write it is made once, on the op's first run, and
// every replay gets the recorded response; in a GET it is made every time.
// The core reads a response whole before it answers, so a body is read whole
// here too.
//
// QuickJS has no web APIs of its own, so this brings what `fetch` needs: a
// small `Headers`, a `Response` with its body already read, and UTF-8.

import client from 'wasi:http/client@0.3.0'
import types from 'wasi:http/types@0.3.0'

// A request or a response body with nothing after it: no trailers, no failure.
const ENDED = {tag: 'ok', val: null}

export async function fetch(resource, init = {}) {
  const url = typeof resource === 'string' ? resource : resource.url
  try {
    return await received(url, await client.send(requestOf(url, init)))
  } catch (failed) {
    throw new TypeError(`fetch failed: ${describe(variant(failed))}`)
  }
}

// The request for `url`, its body streamed after it.
function requestOf(url, {method = 'GET', headers, body} = {}) {
  const {scheme, authority, path} = parts(url)
  const [request] = types.Request.new(types.Fields.fromList(new Headers(headers).list()), [bytesOf(body)], Promise.resolve(ENDED), null)
  request.setMethod(methodOf(method))
  request.setScheme({tag: scheme})
  request.setAuthority(authority)
  request.setPathWithQuery(path)
  return request
}

// A response, its body read whole.
async function received(url, response) {
  const headers = new Headers(response.getHeaders().copyAll().map(([name, value]) => [name, decode(value)]))
  const status = response.getStatusCode()
  const [body] = types.Response.consumeBody(response, Promise.resolve({tag: 'ok'}))
  const chunks = []
  for await (const chunk of body) chunks.push(chunk)
  return new Response(concat(chunks), {status, headers, url})
}

// The WIT error a failing import threw: componentize-qjs throws it as an Error carrying it as `payload`.
const variant = (thrown) => thrown?.payload ?? thrown

const describe = (error) => (error?.val ? `${error.tag}: ${error.val}` : (error?.tag ?? String(error?.message ?? error)))

// A URL's scheme, authority and path with its query.
function parts(url) {
  const found = /^(https?):\/\/([^/?#]+)([^#]*)/i.exec(String(url))
  if (!found) throw new TypeError(`fetch takes an absolute http or https URL, not ${url}`)
  return {scheme: found[1].toUpperCase(), authority: found[2], path: found[3] || '/'}
}

const METHODS = ['get', 'head', 'post', 'put', 'delete', 'connect', 'options', 'trace', 'patch']

function methodOf(method) {
  const lower = method.toLowerCase()
  return METHODS.includes(lower) ? {tag: lower} : {tag: 'other', val: method}
}

// The headers of a request or a response, kept in order, names compared without case.
export class Headers {
  #list = []

  constructor(init) {
    const pairs = init instanceof Headers ? [...init.entries()] : Array.isArray(init) ? init : Object.entries(init ?? {})
    for (const [name, value] of pairs) this.append(name, value)
  }

  append(name, value) {
    this.#list.push([String(name).toLowerCase(), String(value)])
  }

  set(name, value) {
    this.delete(name)
    this.append(name, value)
  }

  delete(name) {
    this.#list = this.#list.filter(([kept]) => kept !== String(name).toLowerCase())
  }

  get(name) {
    const values = this.#list.filter(([kept]) => kept === String(name).toLowerCase()).map(([, value]) => value)
    return values.length ? values.join(', ') : null
  }

  has(name) {
    return this.get(name) !== null
  }

  entries() {
    return this.#list.map(([name, value]) => [name, value])[Symbol.iterator]()
  }

  [Symbol.iterator]() {
    return this.entries()
  }

  forEach(each) {
    for (const [name, value] of this.#list) each(value, name, this)
  }

  // As wasi:http takes them: values in UTF-8.
  list() {
    return this.#list.map(([name, value]) => [name, encode(value)])
  }
}

// A response, its body already read whole.
export class Response {
  #body

  constructor(body, {status = 200, headers, url = ''} = {}) {
    this.#body = body
    this.status = status
    this.ok = status >= 200 && status < 300
    this.statusText = ''
    this.headers = new Headers(headers)
    this.url = url
  }

  async bytes() {
    return this.#body.slice()
  }

  async arrayBuffer() {
    return this.#body.slice().buffer
  }

  async text() {
    return decode(this.#body)
  }

  async json() {
    return JSON.parse(decode(this.#body))
  }
}

function bytesOf(body) {
  if (body === undefined || body === null) return new Uint8Array()
  if (typeof body === 'string') return encode(body)
  if (body instanceof Uint8Array) return body
  if (body instanceof ArrayBuffer) return new Uint8Array(body)
  if (ArrayBuffer.isView(body)) return new Uint8Array(body.buffer, body.byteOffset, body.byteLength)
  throw new TypeError('A fetch body is a string, a Uint8Array or an ArrayBuffer')
}

function concat(chunks) {
  const whole = new Uint8Array(chunks.reduce((length, chunk) => length + chunk.length, 0))
  chunks.reduce((at, chunk) => (whole.set(chunk, at), at + chunk.length), 0)
  return whole
}

// UTF-8, which QuickJS leaves to its embedder.
export function encode(text) {
  const bytes = []
  for (const character of text) {
    const point = character.codePointAt(0)
    if (point < 0x80) bytes.push(point)
    else if (point < 0x800) bytes.push(0xc0 | (point >> 6), 0x80 | (point & 0x3f))
    else if (point < 0x10000) bytes.push(0xe0 | (point >> 12), 0x80 | ((point >> 6) & 0x3f), 0x80 | (point & 0x3f))
    else bytes.push(0xf0 | (point >> 18), 0x80 | ((point >> 12) & 0x3f), 0x80 | ((point >> 6) & 0x3f), 0x80 | (point & 0x3f))
  }
  return new Uint8Array(bytes)
}

export function decode(bytes) {
  let text = ''
  for (let at = 0; at < bytes.length; ) {
    const first = bytes[at]
    const length = first < 0x80 ? 1 : first >= 0xf0 ? 4 : first >= 0xe0 ? 3 : first >= 0xc0 ? 2 : 1
    const point = length === 1 ? (first < 0x80 ? first : 0xfffd) : [...bytes.subarray(at + 1, at + length)].reduce((point, byte) => (point << 6) | (byte & 0x3f), first & (0xff >> (length + 1)))
    text += String.fromCodePoint(point)
    at += length
  }
  return text
}
