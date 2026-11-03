// [chat] end to end: a real server on single-node CarbonDB, driven over HTTP.

import {describe, it, beforeEach, afterEach} from 'node:test'
import assert from 'node:assert'
import {mkdtempSync, writeFileSync} from 'node:fs'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {start} from '../src/main.js'

function configFile() {
  const path = join(mkdtempSync(join(tmpdir(), 'chat-server-')), 'carbondb.toml')
  writeFileSync(path, 'system = "data/system.db"\n[sync]\nmode = "none"\n')
  return path
}

let config, server
beforeEach(async () => {
  config = configFile()
  server = await start({config})
})
afterEach(() => server.close())

// A request to the server, answered with its status and parsed JSON body.
async function request(method, path, body) {
  const response = await fetch(`${server.url}${path}`, {
    method,
    headers: {'content-type': 'application/json'},
    body: typeof body === 'string' ? body : body === undefined ? undefined : JSON.stringify(body)
  })
  return {status: response.status, body: await response.json()}
}

const messages = async () => (await request('GET', '/chat/messages')).body

describe('messages', () => {
  it('posts, lists, edits and deletes a message', async () => {
    const posted = await request('POST', '/chat/messages', {text: 'hello'})
    assert.strictEqual(posted.status, 200)
    const {id} = posted.body

    const [listed] = await messages()
    assert.deepStrictEqual([listed.id, listed.text, listed.edited_at], [id, 'hello', null])
    assert.match(listed.author, /^[0-9a-f]{64}$/)

    assert.deepStrictEqual(await request('PUT', `/chat/messages/${id}`, {text: 'hello, edited'}), {status: 200, body: {id}})
    const [edited] = await messages()
    assert.strictEqual(edited.text, 'hello, edited')
    assert.ok(edited.edited_at >= listed.sent_at)

    assert.deepStrictEqual(await request('DELETE', `/chat/messages/${id}`), {status: 200, body: {id, deleted: true}})
    assert.deepStrictEqual(await messages(), [])
  })

  it('lists messages in the order they were sent', async () => {
    for (const text of ['one', 'two', 'three']) await request('POST', '/chat/messages', {text})
    assert.deepStrictEqual((await messages()).map((message) => message.text), ['one', 'two', 'three'])
  })

  it('keeps messages and the identity across a restart', async () => {
    await request('POST', '/chat/messages', {text: 'still here'})
    const [before] = await messages()
    await server.close()
    server = await start({config})
    const [after] = await messages()
    assert.deepStrictEqual(after, before)
  })
})

describe('errors', () => {
  const cases = [
    {why: 'an empty message', method: 'POST', path: '/chat/messages', body: {text: '  '}, status: 422, code: 'VALIDATION_ERROR'},
    {why: 'a message with no text', method: 'POST', path: '/chat/messages', body: {}, status: 422, code: 'VALIDATION_ERROR'},
    {why: 'a message with unknown fields', method: 'POST', path: '/chat/messages', body: {text: 'x', pinned: true}, status: 422, code: 'VALIDATION_ERROR'},
    {why: 'a body that is not JSON', method: 'POST', path: '/chat/messages', body: '{not json', status: 400, code: 'BAD_REQUEST'},
    {why: 'editing a message that does not exist', method: 'PUT', path: '/chat/messages/nope', body: {text: 'x'}, status: 404, code: 'NOT_FOUND'},
    {why: 'deleting a message that does not exist', method: 'DELETE', path: '/chat/messages/nope', status: 404, code: 'NOT_FOUND'},
    {why: 'a method the resource does not answer', method: 'PATCH', path: '/chat/messages', body: {}, status: 405, code: 'METHOD_NOT_ALLOWED'},
    {why: 'a service the space does not run', method: 'GET', path: '/contacts/people', status: 404, code: 'NOT_FOUND'},
    {why: 'a call that needs peers', method: 'POST', path: '/@carbon/members', body: {contact: 'x'}, status: 501, code: 'SINGLE_NODE'}
  ]

  for (const {why, method, path, body, status, code} of cases) {
    it(`answers ${why} with ${status} ${code}`, async () => {
      const response = await request(method, path, body)
      assert.deepStrictEqual([response.status, response.body.code], [status, code])
      assert.strictEqual(response.body.status, status)
    })
  }

  it('reports what failed validation in the details', async () => {
    const {body} = await request('POST', '/chat/messages', {})
    assert.deepStrictEqual(body.details.map((issue) => issue.path), [['text']])
  })

  it('leaves no message behind after a refused call', async () => {
    await request('POST', '/chat/messages', {text: ''})
    assert.deepStrictEqual(await messages(), [])
  })
})

describe('status', () => {
  it('describes the lobby: its name, services and single-node state', async () => {
    const {status, body} = await request('GET', '/@carbon/status')
    assert.strictEqual(status, 200)
    assert.deepStrictEqual(
      {kind: body.kind, name: body.name, services: body.services, access: body.access, sync: body.sync, peers: body.peers},
      {kind: 'space', name: 'Lobby', services: ['chat'], access: ['manage'], sync: 'off', peers: []}
    )
  })
})

describe('watch', () => {
  // The events of a server-sent event stream, one at a time.
  async function* events(path) {
    const response = await fetch(`${server.url}${path}`)
    const decoder = new TextDecoder()
    let buffered = ''
    for await (const chunk of response.body) {
      buffered += decoder.decode(chunk, {stream: true})
      const frames = buffered.split('\n\n')
      buffered = frames.pop()
      for (const frame of frames) yield JSON.parse(frame.replace(/^data: /, ''))
    }
  }

  it('streams the list again after each message is posted', async () => {
    const stream = events('/watch/chat/messages')
    assert.deepStrictEqual((await stream.next()).value, {value: []})
    await request('POST', '/chat/messages', {text: 'live'})
    const {value} = (await stream.next()).value
    assert.deepStrictEqual(value.map((message) => message.text), ['live'])
    await stream.return()
  })
})
